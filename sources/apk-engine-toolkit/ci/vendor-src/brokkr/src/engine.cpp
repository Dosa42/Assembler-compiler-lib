#include "brokkr_engine.h"
#include "app/md5_verify.hpp"
#include "app/pit_file.hpp"
#include "io/lz4_frame.hpp"
#include "io/read_exact.hpp"
#include "io/tar.hpp"
#include "third_party/lz4/lz4.h"
#include "third_party/xxhash/xxhash_vendor.h"
#include "platform/android/libusb_transport.hpp"
#include "protocol/odin/flash.hpp"
#include "protocol/odin/group_flasher.hpp"
#include "protocol/odin/pit_transfer.hpp"
#include "brokkr_usb_client.h"
#include <libusb.h>
#include <spdlog/spdlog.h>
#include <algorithm>
#include <atomic>
#include <cerrno>
#include <climits>
#include <cstdlib>
#include <cstring>
#include <fcntl.h>
#include <filesystem>
#include <mutex>
#include <stdexcept>
#include <string>
#include <string_view>
#include <unistd.h>
#include <vector>

namespace {
using namespace brokkr;
using std::string;

struct Failure : std::runtime_error {
  string code;
  int exit_code;
  Failure(string c, string message, int status = 1)
      : std::runtime_error(std::move(message)), code(std::move(c)), exit_code(status) {}
};

template <class T> T required(std::expected<T, string> value, const char *code = "operation_failed") {
  if (!value) throw Failure(code, value.error());
  return std::move(*value);
}
void required(core::Status status, const char *code = "operation_failed") {
  if (!status) throw Failure(code, status.error());
}

string quote(std::string_view value) {
  const char hex[] = "0123456789abcdef";
  string out = "\"";
  for (std::size_t i = 0; i < value.size(); ++i) {
    const unsigned char c = value[i];
    if (c == '"' || c == '\\') { out += '\\'; out += c; }
    else if (c < 0x20) {
      out += "\\u00"; out += hex[c >> 4]; out += hex[c & 15];
    } else if (c < 0x80) out += c;
    else {
      std::size_t n = (c >= 0xc2 && c <= 0xdf) ? 2 : (c >= 0xe0 && c <= 0xef) ? 3 : (c >= 0xf0 && c <= 0xf4) ? 4 : 0;
      bool valid = n && i + n <= value.size();
      for (std::size_t j = 1; valid && j < n; ++j) valid = (static_cast<unsigned char>(value[i + j]) & 0xc0) == 0x80;
      if (valid && n >= 3) {
        const unsigned char next = value[i + 1];
        valid = !((c == 0xe0 && next < 0xa0) || (c == 0xed && next >= 0xa0) ||
                  (c == 0xf0 && next < 0x90) || (c == 0xf4 && next >= 0x90));
      }
      if (valid) { out.append(value.substr(i, n)); i += n - 1; }
      else { out += "\\u00"; out += hex[c >> 4]; out += hex[c & 15]; }
    }
  }
  return out + '"';
}
template <class T> string number(T value) { return std::to_string(value); }
string boolean(bool value) { return value ? "true" : "false"; }
string object(std::initializer_list<std::pair<string, string>> fields) {
  string out = "{";
  for (const auto &[key, value] : fields) { if (out.size() > 1) out += ','; out += quote(key) + ':' + value; }
  return out + '}';
}
template <class Range, class Fn> string array(const Range &values, Fn emit) {
  string out = "[";
  for (const auto &value : values) { if (out.size() > 1) out += ','; out += emit(value); }
  return out + ']';
}

odin::pit::PitTable read_pit(const string &path) {
  std::error_code ec;
  const auto bytes = std::filesystem::file_size(path, ec);
  if (ec) throw Failure("input_error", "Cannot stat PIT: " + path);
  if (bytes > 16 * 1024 * 1024) throw Failure("input_error", "PIT exceeds 16 MiB limit");
  auto data = required(app::read_pit_file(path), "input_error");
  return required(odin::pit::parse(data), "invalid_pit");
}
string pit_json(const odin::pit::PitTable &pit) {
  auto common = pit.common_block_size();
  return object({{"com_tar2", quote(pit.com_tar2)}, {"cpu_bl_id", quote(pit.cpu_bl_id)},
    {"lu_count", number(pit.lu_count)}, {"is_ab", boolean(pit.is_ab())},
    {"common_block_bytes", common ? number(*common) : "null"},
    {"partitions", array(pit.partitions, [](const auto &p) {
      return object({{"id", number(p.id)}, {"name", quote(p.name)}, {"file_name", quote(p.file_name)},
        {"bin_type", number(p.bin_type)}, {"dev_type", number(p.dev_type)}, {"attribute", number(p.attribute)},
        {"begin_block", number(p.begin_block)}, {"block_bytes", number(p.block_bytes)},
        {"block_count", number(p.block_size)}, {"capacity_bytes", number(p.file_size)},
        {"wire_block_size", number(p.wire_block_size)}});
    })}});
}
string with_path(const string &path, string body) { return "{\"path\":" + quote(path) + ',' + body.substr(1); }

std::vector<io::RandomAccessSourcePtr> open_sources(const std::vector<string> &paths) {
  std::vector<io::RandomAccessSourcePtr> sources;
  for (const auto &path : paths) sources.push_back(required(io::open_file_source(path), "input_error"));
  return sources;
}
std::vector<odin::ImageSpec> expand(const std::vector<io::RandomAccessSourcePtr> &sources) {
  auto images = required(odin::expand_inputs(sources), "invalid_firmware");
  if (images.empty()) throw Failure("invalid_firmware", "No firmware images found");
  return images;
}
string image_json(const odin::ImageSpec &image) {
  return object({{"basename", quote(image.basename)}, {"source_basename", quote(image.source_basename)},
    {"size_bytes", number(image.size)}, {"disk_bytes", number(image.disk_size)}, {"lz4", boolean(image.lz4)},
    {"lz4_block_bytes", number(image.lz4_block_size)}, {"display", quote(image.display)}});
}

std::vector<app::Md5Job> md5_jobs(const std::vector<io::RandomAccessSourcePtr> &sources) {
  auto jobs = required(app::md5_jobs_from_sources(sources), "invalid_md5");
  for (const auto &source : sources) {
    string label = source->label();
    std::transform(label.begin(), label.end(), label.begin(), [](unsigned char c) { return std::tolower(c); });
    if (label.ends_with(".md5") && std::none_of(jobs.begin(), jobs.end(), [&](const auto &job) { return job.source == source; }))
      throw Failure("missing_md5", "MD5-wrapped firmware has no valid checksum trailer: " + source->label());
  }
  return jobs;
}
void validate_image_reads(const std::vector<odin::ImageSpec> &images) {
  for (const auto &image : images) {
    if (!image.lz4) continue;
    auto header_source = required(image.open(), "invalid_firmware");
    const auto header = required(io::parse_lz4_frame_header(*header_source), "invalid_lz4");
    auto source = required(image.open(), "invalid_firmware");
    std::array<std::byte, 15> raw_header{};
    required(io::read_exact(*source, raw_header), "invalid_lz4");
    if ((header.flg & 2) || (header.bd & 0x8f) ||
        ((XXH32(raw_header.data() + 4, 10, 0) >> 8) & 255) != std::to_integer<unsigned>(raw_header[14]))
      throw Failure("invalid_lz4", "LZ4 header checksum or reserved bits invalid");
    auto read_u32 = [&]() {
      std::array<std::byte, 4> data{};
      required(io::read_exact(*source, data), "invalid_lz4");
      std::uint32_t value = 0;
      for (unsigned i = 0; i < 4; ++i) value |= std::to_integer<std::uint32_t>(data[i]) << (8 * i);
      return value;
    };
    std::unique_ptr<XXH32_state_t, decltype(&XXH32_freeState)> hash(XXH32_createState(), XXH32_freeState);
    if (!hash || XXH32_reset(hash.get(), 0) != XXH_OK) throw Failure("allocation", "Cannot initialize LZ4 content hash");
    std::vector<std::byte> compressed(header.max_block_size), decompressed(header.max_block_size);
    std::uint64_t bytes = 0;
    std::uint64_t consumed = raw_header.size();
    for (;;) {
      const auto block = read_u32();
      consumed += 4;
      if (!block) break;
      const auto size = block & 0x7fffffffu;
      if (!size || size > header.max_block_size || bytes >= header.content_size)
        throw Failure("invalid_lz4", "Invalid LZ4 block length");
      required(io::read_exact(*source, {compressed.data(), size}), "invalid_lz4");
      consumed += size;
      const auto expected = static_cast<std::size_t>(std::min<std::uint64_t>(header.max_block_size, header.content_size - bytes));
      const bool raw = (block & 0x80000000u) != 0;
      const int decoded = raw ? static_cast<int>(size) : LZ4_decompress_safe(
        reinterpret_cast<const char *>(compressed.data()), reinterpret_cast<char *>(decompressed.data()),
        static_cast<int>(size), static_cast<int>(expected));
      if (decoded < 0 || static_cast<std::size_t>(decoded) != expected)
        throw Failure("invalid_lz4", "LZ4 decompressed block length mismatch");
      if (XXH32_update(hash.get(), raw ? compressed.data() : decompressed.data(), expected) != XXH_OK)
        throw Failure("invalid_lz4", "LZ4 content hash failed");
      bytes += expected;
    }
    if (bytes != image.size) throw Failure("invalid_lz4", "Decompressed image length mismatch");
    if (header.content_checksum) {
      const auto expected = read_u32();
      consumed += 4;
      if (expected != XXH32_digest(hash.get())) throw Failure("invalid_lz4", "LZ4 content checksum mismatch");
    }
    if (consumed != image.disk_size) throw Failure("invalid_lz4", "Unexpected bytes after LZ4 frame");
  }
}

string inspect(const string &path) {
  auto sources = open_sources({path});
  auto images = expand(sources);
  const bool is_tar = io::TarArchive::is_tar_file(*sources.front());
  string entries = "[]", payload = "null";
  if (is_tar) {
    auto tar = required(io::TarArchive::open(sources.front(), true), "invalid_firmware");
    entries = array(tar.entries(), [](const auto &entry) {
      return object({{"name", quote(entry.name)}, {"size_bytes", number(entry.size)}, {"data_offset", number(entry.data_offset)}});
    });
    if (auto n = tar.payload_size_bytes()) payload = number(*n);
  }
  return object({{"path", quote(path)}, {"file_bytes", number(sources.front()->size())},
    {"format", quote(is_tar ? "tar" : "raw")}, {"tar_payload_bytes", payload}, {"entries", entries},
    {"images", array(images, image_json)}});
}
string verify(const string &path) {
  auto sources = open_sources({path});
  auto images = expand(sources);
  auto jobs = md5_jobs(sources);
  app::clear_session_verify_cache();
  required(app::md5_verify(jobs, {}), "checksum_mismatch");
  validate_image_reads(images);
  std::uint64_t bytes = 0;
  for (const auto &job : jobs) bytes += job.bytes_to_hash;
  return object({{"path", quote(path)}, {"structural_valid", "true"},
    {"md5_status", quote(jobs.empty() ? "absent" : "verified")}, {"md5_verified", boolean(!jobs.empty())},
    {"bytes_hashed", number(bytes)}, {"images_checked", number(images.size())}});
}
string map_images(const string &pit_path, const std::vector<string> &paths) {
  const auto pit = read_pit(pit_path);
  auto images = expand(open_sources(paths));
  auto items = required(odin::map_to_pit(pit, images), "mapping_failed");
  if (items.empty()) throw Failure("mapping_failed", "No firmware images match the PIT");
  std::vector<string> unmapped;
  for (const auto &image : images) if (!pit.find_by_file_name(image.basename)) unmapped.push_back(image.basename);
  return object({{"pit", quote(pit_path)}, {"items", array(items, [](const auto &item) {
    return object({{"partition_id", number(item.part.id)}, {"partition_name", quote(item.part.name)},
      {"dev_type", number(item.part.dev_type)}, {"image", quote(item.spec.basename)},
      {"size_bytes", number(item.spec.size)}, {"capacity_bytes", number(item.part.file_size)}});
  })}, {"unmapped", array(unmapped, [](const auto &name) { return quote(name); })}});
}

struct OwnedFd {
  int fd = -1;
  ~OwnedFd() { if (fd >= 0) ::close(fd); }
};
struct UsbOptions {
  int fd = -1, interface = -1, in = -1, out = -1;
  string socket;
  bool reboot = false;
  std::vector<string> paths;
};
int integer(const string &text) {
  char *end = nullptr;
  errno = 0;
  const long value = std::strtol(text.c_str(), &end, 0);
  if (errno || end == text.c_str() || *end || value < 0 || value > INT_MAX)
    throw Failure("usage", "Expected nonnegative integer: " + text, 2);
  return static_cast<int>(value);
}
UsbOptions usb_options(const std::vector<string> &args, bool flash) {
  UsbOptions options;
  std::vector<string> seen;
  for (std::size_t i = 1; i < args.size(); ++i) {
    const auto &key = args[i];
    if (key != "--ap" && std::find(seen.begin(), seen.end(), key) != seen.end()) throw Failure("usage", "Duplicate option: " + key, 2);
    seen.push_back(key);
    if (key == "--reboot" && flash) { options.reboot = true; continue; }
    if (i + 1 == args.size()) throw Failure("usage", "Missing option value: " + key, 2);
    const auto &value = args[++i];
    if (key == "--fd") options.fd = integer(value);
    else if (key == "--socket") options.socket = value;
    else if (key == "--interface") options.interface = integer(value);
    else if (key == "--in") options.in = integer(value);
    else if (key == "--out") options.out = integer(value);
    else if (key == "--ap" && flash) options.paths.push_back(value);
    else throw Failure("usage", "Unknown option: " + key, 2);
  }
  if ((options.fd < 0) == options.socket.empty()) throw Failure("usage", "Supply exactly one of --fd or --socket", 2);
  if (flash && options.paths.empty()) throw Failure("usage", "flash requires --ap FILE", 2);
  if ((options.interface >= 0 || options.in >= 0 || options.out >= 0) &&
      !(options.interface >= 0 && options.in >= 0 && options.out >= 0))
    throw Failure("usage", "Supply --interface, --in and --out together", 2);
  if (options.interface > 255 || options.in > 255 || options.out > 255 || options.in == 128 || options.out == 0 ||
      (options.in >= 0 && !(options.in & 128)) || (options.out >= 0 && (options.out & 128)))
    throw Failure("usage", "Invalid USB interface or bulk endpoints", 2);
  return options;
}
void discover_endpoints(android_platform::LibusbContext &context, UsbOptions &options) {
  if (options.interface >= 0) return;
  libusb_device_handle *raw = nullptr;
  int rc = libusb_wrap_sys_device(context.raw(), options.fd, &raw);
  if (rc || !raw) throw Failure("usb_open", "Descriptor inspection failed: " + string(libusb_error_name(rc)));
  std::unique_ptr<libusb_device_handle, decltype(&libusb_close)> handle(raw, libusb_close);
  libusb_config_descriptor *config_raw = nullptr;
  rc = libusb_get_active_config_descriptor(libusb_get_device(raw), &config_raw);
  if (rc || !config_raw) throw Failure("usb_descriptors", "Active USB descriptors unavailable: " + string(libusb_error_name(rc)));
  std::unique_ptr<libusb_config_descriptor, decltype(&libusb_free_config_descriptor)> config(config_raw, libusb_free_config_descriptor);
  std::vector<UsbOptions> candidates;
  for (int i = 0; i < config->bNumInterfaces; ++i) {
    for (int j = 0; j < config->interface[i].num_altsetting; ++j) {
      const auto &alt = config->interface[i].altsetting[j];
      if (alt.bAlternateSetting != 0) continue;
      int in = -1, out = -1, inputs = 0, outputs = 0;
      for (int k = 0; k < alt.bNumEndpoints; ++k) {
        const auto &ep = alt.endpoint[k];
        if ((ep.bmAttributes & LIBUSB_TRANSFER_TYPE_MASK) != LIBUSB_TRANSFER_TYPE_BULK) continue;
        if (ep.bEndpointAddress & LIBUSB_ENDPOINT_IN) { in = ep.bEndpointAddress; ++inputs; }
        else { out = ep.bEndpointAddress; ++outputs; }
      }
      if (inputs == 1 && outputs == 1) {
        UsbOptions match; match.interface = alt.bInterfaceNumber; match.in = in; match.out = out;
        candidates.push_back(std::move(match));
      }
    }
  }
  if (candidates.size() != 1) throw Failure("usb_ambiguous", "Supply --interface, --in and --out: expected exactly one bulk interface");
  options.interface = candidates.front().interface;
  options.in = candidates.front().in;
  options.out = candidates.front().out;
}
string device(const std::vector<string> &args, bool flash) {
  auto options = usb_options(args, flash);
  std::vector<odin::ImageSpec> images;
  std::shared_ptr<const std::vector<std::byte>> pit_to_upload;
  if (flash) {
    auto sources = open_sources(options.paths);
    images = expand(sources);
    app::clear_session_verify_cache();
    required(app::md5_verify(required(app::md5_jobs_from_sources(sources), "invalid_md5"), {}), "checksum_mismatch");
    pit_to_upload = odin::pit_from_specs(images);
    const bool has_pit = std::any_of(images.begin(), images.end(), [](const auto &image) { return odin::is_pit_name(image.basename); });
    if (has_pit && !pit_to_upload) throw Failure("invalid_pit", "Cannot read embedded PIT");
    std::erase_if(images, [](const auto &image) { return odin::is_pit_name(image.basename); });
    if (images.empty()) throw Failure("invalid_firmware", "No flashable firmware images");
  }
  OwnedFd acquired;
  if (!options.socket.empty()) {
    char metadata[8192] = {};
    acquired.fd = brokkr_usb_acquire(options.socket.c_str(), metadata, sizeof(metadata));
    if (acquired.fd < 0) throw Failure("usb_session", "USB broker acquisition failed: " + string(std::strerror(errno)));
    options.fd = acquired.fd;
  }
  if (::fcntl(options.fd, F_GETFD) < 0) throw Failure("usb_fd", "USB descriptor is not open in this process");
  auto context = required(android_platform::LibusbContext::create(), "usb_context");
  discover_endpoints(*context, options);
  auto transport = required(android_platform::LibusbUsbTransport::open({
    .context = context.get(), .fd = options.fd, .interface_number = options.interface,
    .bulk_in_addr = static_cast<std::uint8_t>(options.in), .bulk_out_addr = static_cast<std::uint8_t>(options.out),
    .id = "native-usb"}), "usb_open");
  transport->set_timeout_ms(1000);
  if (!flash) {
    odin::OdinCommands commands(*transport);
    required(commands.handshake(2), "handshake_failed");
    const auto info = required(commands.get_version(2), "protocol_failed");
    if (info.protocol() >= odin::ProtocolVersion::PROTOCOL_VER2)
      required(commands.setup_transfer_options(1024 * 1024, 2), "protocol_failed");
    const auto pit = required(odin::download_pit_table(commands, 2), "pit_download_failed");
    required(commands.shutdown(false, 2), "session_close_failed");
    return object({{"protocol", number(static_cast<int>(info.protocol()))}, {"ack_word", number(info.ack_word)},
      {"compressed_download", boolean(info.supports_compressed_download())},
      {"interface", number(options.interface)}, {"bulk_in", number(options.in)}, {"bulk_out", number(options.out)}, {"pit", pit_json(pit)}});
  }
  odin::Target target{.id = "native-usb", .link = transport.get()};
  std::vector<odin::Target *> targets{&target};
  odin::Cfg config;
  config.reboot_after = options.reboot;
  config.step_delay_ms = 0;
  config.post_close_delay_ms = 0;
  odin::Ui ui;
  std::size_t planned_images = 0;
  ui.on_plan = [&](const std::vector<odin::PlanItem> &plan, std::uint64_t) {
    planned_images = std::count_if(plan.begin(), plan.end(), [](const auto &item) { return item.kind == odin::PlanItem::Kind::Part; });
  };
  ui.on_stage = [](const string &stage) { std::fprintf(stderr, "Brokkr: %s\n", stage.c_str()); };
  ui.on_error = [](const string &message) { std::fprintf(stderr, "Brokkr: %s\n", message.c_str()); };
  required(odin::flash(targets, images, pit_to_upload, config, ui), "flash_failed");
  return object({{"completed", "true"}, {"reboot_requested", boolean(options.reboot)},
    {"embedded_pit_uploaded", boolean(static_cast<bool>(pit_to_upload))}, {"images", number(planned_images)}});
}

string dispatch(const std::vector<string> &args) {
  if (args.empty()) throw Failure("usage", "Expected operation: parse-pit, inspect-firmware, verify-firmware, map-firmware-to-pit, device-probe, flash", 2);
  const auto &op = args.front();
  if (op == "device-probe" || op == "flash") return device(args, op == "flash");
  if (op == "map-firmware-to-pit") {
    if (args.size() < 3) throw Failure("usage", "map-firmware-to-pit PIT FILE...", 2);
    return map_images(args[1], {args.begin() + 2, args.end()});
  }
  if (op != "parse-pit" && op != "inspect-firmware" && op != "verify-firmware") throw Failure("usage", "Unknown operation: " + op, 2);
  if (args.size() != 2) throw Failure("usage", op + " FILE", 2);
  if (op == "parse-pit") return with_path(args[1], pit_json(read_pit(args[1])));
  if (op == "inspect-firmware") return inspect(args[1]);
  return verify(args[1]);
}
}

extern "C" int brokkr_execute(int argc, const char *const *argv, char **output_json) {
  if (!output_json) return 2;
  *output_json = nullptr;
  static std::mutex mutex;
  std::lock_guard lock(mutex);
  int code = 0;
  string operation = "unknown", output;
  try {
    spdlog::set_level(spdlog::level::off);
    if (argc < 0 || argc > 4096 || (argc && !argv)) throw Failure("usage", "Invalid argument vector", 2);
    std::vector<string> args;
    for (int i = 0; i < argc; ++i) {
      if (!argv[i]) throw Failure("usage", "Null argument", 2);
      args.emplace_back(argv[i]);
    }
    if (!args.empty()) operation = args.front();
    auto result = dispatch(args);
    output = object({{"schema", quote("brokkr.result/1")}, {"ok", "true"}, {"operation", quote(operation)}, {"result", result}});
  } catch (const Failure &error) {
    code = error.exit_code;
    output = object({{"schema", quote("brokkr.result/1")}, {"ok", "false"}, {"operation", quote(operation)},
      {"error", object({{"code", quote(error.code)}, {"message", quote(error.what())}})}});
  } catch (const std::exception &error) {
    code = 1;
    output = object({{"schema", quote("brokkr.result/1")}, {"ok", "false"}, {"operation", quote(operation)},
      {"error", object({{"code", quote("exception")}, {"message", quote(error.what())}})}});
  } catch (...) { return 70; }
  *output_json = static_cast<char *>(std::malloc(output.size() + 1));
  if (!*output_json) return 70;
  std::memcpy(*output_json, output.c_str(), output.size() + 1);
  return code;
}

extern "C" void brokkr_free(char *output_json) { std::free(output_json); }
