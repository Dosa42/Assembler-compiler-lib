package dev.ayrvokan.apkengine;

import com.google.gson.*;
import com.iyxan23.zipalignjava.ZipAlign;
import net.lingala.zip4j.ZipFile;
import net.lingala.zip4j.model.FileHeader;
import net.lingala.zip4j.model.ZipParameters;
import net.lingala.zip4j.model.enums.CompressionMethod;
import java.io.*;
import java.nio.file.*;
import java.util.*;
import static dev.ayrvokan.apkengine.Main.*;

final class ArchiveTools {
    static void register() {
        tool("inspect_apk", "zip4j", "Inspect APK ZIP entries, DEX files and native library paths.", false, new String[]{"path"},
                obj("path", field("string", "APK path")), ArchiveTools::inspect);
        tool("list_entries", "zip4j", "List a page of APK or ZIP entries with sizes and CRC32.", false, new String[]{"path"},
                obj("path", field("string", "APK or ZIP path"), "prefix", field("string", "Entry prefix filter"), "offset", field("integer", "Page offset, default 0"), "limit", field("integer", "Page size 1..1000, default 100")), ArchiveTools::list);
        tool("extract_entries", "zip4j", "Extract all or selected ZIP entries into an empty directory.", true, new String[]{"path", "output_dir"},
                obj("path", field("string", "APK or ZIP path"), "output_dir", field("string", "Empty destination directory"), "entries", obj("type", "array", "items", field("string", "Exact entry name"), "description", "Entries to extract; omit for all entries")), ArchiveTools::extract);
        tool("create_archive", "zip4j", "Create a ZIP from a directory, preserving relative paths.", true, new String[]{"source_dir", "output"},
                obj("source_dir", field("string", "Source directory"), "output", field("string", "Output ZIP path"), "store", field("boolean", "Store without compression, default false"), "overwrite", field("boolean", "Replace an existing output, default false")), ArchiveTools::create);
        tool("align_apk", "zipalign-java", "Align stored APK entries to 4 bytes and native .so entries to 16 KiB; run before signing.", true, new String[]{"path", "output"},
                obj("path", field("string", "Input APK or ZIP"), "output", field("string", "Output aligned file"), "overwrite", field("boolean", "Replace an existing output, default false")), ArchiveTools::align);
        tool("check_alignment", "zip4j", "Check actual ZIP local data offsets for 4-byte and 16 KiB alignment.", false, new String[]{"path"},
                obj("path", field("string", "APK or ZIP path")), a -> alignment(input(a, "path")));
    }

    static JsonObject inspect(JsonObject args) throws Exception {
        Path path = input(args, "path");
        try (ZipFile zip = new ZipFile(path.toFile())) {
            List<FileHeader> headers = zip.getFileHeaders();
            List<String> dex = new ArrayList<>(), nativeLibs = new ArrayList<>();
            long expanded = 0;
            boolean manifest = false, resources = false;
            for (FileHeader h : headers) {
                expanded += h.getUncompressedSize();
                String name = h.getFileName();
                if (name.matches("classes[0-9]*\\.dex")) dex.add(name);
                if (name.startsWith("lib/") && name.endsWith(".so")) nativeLibs.add(name);
                manifest |= name.equals("AndroidManifest.xml");
                resources |= name.equals("resources.arsc");
            }
            return obj("artifact", artifact(path), "entries", headers.size(), "expanded_bytes", expanded,
                    "manifest", manifest, "resources_arsc", resources, "dex", dex, "native_libraries", nativeLibs);
        }
    }

    static JsonObject list(JsonObject args) throws Exception {
        Path path = input(args, "path");
        String prefix = args.has("prefix") ? string(args, "prefix") : "";
        int offset = integer(args, "offset", 0, 0, Integer.MAX_VALUE), limit = integer(args, "limit", 100, 1, 1000);
        try (ZipFile zip = new ZipFile(path.toFile())) {
            List<FileHeader> selected = zip.getFileHeaders().stream().filter(h -> h.getFileName().startsWith(prefix)).toList();
            JsonArray entries = new JsonArray();
            int end = (int)Math.min(selected.size(), (long)offset + limit);
            for (int i = offset; i < end; i++) {
                FileHeader h = selected.get(i);
                entries.add(obj("name", h.getFileName(), "bytes", h.getUncompressedSize(), "compressed_bytes", h.getCompressedSize(),
                        "crc32", String.format("%08x", h.getCrc()), "directory", h.isDirectory(), "encrypted", h.isEncrypted(), "compression", h.getCompressionMethod().toString()));
            }
            return obj("path", path, "total", selected.size(), "offset", offset, "entries", entries, "next_offset", end < selected.size() ? end : null);
        }
    }

    static JsonObject extract(JsonObject args) throws Exception {
        Path path = input(args, "path");
        Path dest = destination(args);
        try (ZipFile zip = new ZipFile(path.toFile())) {
            List<String> names = new ArrayList<>();
            if (args.has("entries")) {
                for (JsonElement entry : args.getAsJsonArray("entries")) {
                    if (!entry.isJsonPrimitive() || !entry.getAsJsonPrimitive().isString()) throw new IllegalArgumentException("entries must contain strings");
                    String name = entry.getAsString();
                    if (zip.getFileHeader(name) == null) throw new FileNotFoundException("Entry not found: " + name);
                    names.add(name);
                }
                for (String name : names) zip.extractFile(name, dest.toString());
            } else {
                for (FileHeader h : zip.getFileHeaders()) names.add(h.getFileName());
                zip.extractAll(dest.toString());
            }
            return obj("path", path, "output_dir", dest, "entry_count", names.size());
        }
    }

    static JsonObject create(JsonObject args) throws Exception {
        Path source = Path.of(string(args, "source_dir")).toAbsolutePath().normalize().toRealPath();
        if (!Files.isDirectory(source)) throw new IOException("Source must be a directory");
        Path target = output(args, "output");
        if (target.startsWith(source)) throw new IOException("Output must be outside the source directory");
        return writeArtifact(args, target, temporary -> {
            ZipParameters parameters = new ZipParameters();
            parameters.setIncludeRootFolder(false);
            parameters.setCompressionMethod(bool(args, "store", false) ? CompressionMethod.STORE : CompressionMethod.DEFLATE);
            try (ZipFile zip = new ZipFile(temporary.toFile())) { zip.addFolder(source.toFile(), parameters); }
        });
    }

    static JsonObject align(JsonObject args) throws Exception {
        Path source = input(args, "path"), target = output(args, "output");
        different(source, target);
        JsonObject result = writeArtifact(args, target, temporary -> {
            try (RandomAccessFile in = new RandomAccessFile(source.toFile(), "r"); OutputStream out = Files.newOutputStream(temporary)) {
                ZipAlign.alignZip(in, out, 4, 16384);
            }
            if (!alignment(temporary).get("aligned").getAsBoolean()) throw new IOException("Alignment verification failed");
        });
        result.add("alignment", alignment(target));
        return result;
    }

    static int little16(RandomAccessFile in) throws IOException {
        int a = in.read(), b = in.read();
        if (a < 0 || b < 0) throw new EOFException();
        return a | b << 8;
    }

    static JsonObject alignment(Path path) throws Exception {
        JsonArray failures = new JsonArray();
        int checked = 0;
        try (ZipFile zip = new ZipFile(path.toFile()); RandomAccessFile in = new RandomAccessFile(path.toFile(), "r")) {
            for (FileHeader h : zip.getFileHeaders()) {
                if (h.isDirectory() || h.getCompressionMethod() != CompressionMethod.STORE) continue;
                long header = h.getOffsetLocalHeader();
                in.seek(header);
                if (Integer.reverseBytes(in.readInt()) != 0x04034b50) throw new IOException("Invalid local ZIP header: " + h.getFileName());
                in.seek(header + 26);
                int name = little16(in), extra = little16(in);
                long offset = header + 30 + name + extra;
                int required = h.getFileName().endsWith(".so") ? 16384 : 4;
                checked++;
                if (offset % required != 0) failures.add(obj("entry", h.getFileName(), "data_offset", offset, "required_alignment", required));
            }
        }
        return obj("path", path, "aligned", failures.isEmpty(), "stored_entries_checked", checked, "failures", failures);
    }
}
