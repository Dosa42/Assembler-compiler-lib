package dev.ayrvokan.apkengine;

import com.google.gson.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.*;
import static dev.ayrvokan.apkengine.Main.*;

final class BrokkrTools {
    static final int OUTPUT_LIMIT = 16 * 1024 * 1024;
    static final String ENGINE_ENV = "APK_ENGINE_BROKKR";
    static final String BROKER_ENV = "APK_ENGINE_BROKKR_USB_BROKER";
    static final String USB_ENV = "APK_ENGINE_TERMUX_USB";
    static final JsonObject RESULT_SCHEMA = resultSchema();
    record Capture(String text, int bytes) {}
    record Execution(int exitCode, String stdout, String stderr) {}
    static final class NativeFailure extends IOException {
        final JsonObject result;
        final int exitCode;
        NativeFailure(String message, JsonObject result, int exitCode) {
            super(message);
            this.result = result;
            this.exitCode = exitCode;
        }
    }

    static JsonObject resultSchema() {
        try (InputStream input = BrokkrTools.class.getResourceAsStream("/brokkr-result.schema.json")) {
            if (input == null) throw new IOException("Missing embedded Brokkr result schema");
            return parseObject(new String(input.readAllBytes(), StandardCharsets.UTF_8));
        } catch (IOException error) { throw new ExceptionInInitializerError(error); }
    }

    static JsonObject arrayField(String description) {
        return obj("type", "array", "description", description, "items", obj("type", "string", "minLength", 1), "minItems", 1);
    }

    static JsonObject socketProperties() {
        return obj("socket", field("string", "Absolute path to the live Brokkr USB broker socket; defaults to the Termux temporary directory"));
    }

    static JsonObject deviceProperties() {
        JsonObject properties = socketProperties();
        properties.add("usb_interface", obj("type", "integer", "minimum", 0, "maximum", 255, "description", "USB interface number"));
        properties.add("in_endpoint", obj("type", "integer", "minimum", 128, "maximum", 255, "description", "Bulk IN endpoint address, including direction bit"));
        properties.add("out_endpoint", obj("type", "integer", "minimum", 1, "maximum", 127, "description", "Bulk OUT endpoint address"));
        return properties;
    }

    static JsonObject outputSchema(String name, String nativeSchema) {
        JsonObject nativeProperties = obj("schema", obj("type", "string", "const", nativeSchema));
        List<String> required = new ArrayList<>(List.of("schema"));
        if (nativeSchema.equals("brokkr.result/1")) {
            nativeProperties.add("ok", obj("type", "boolean", "const", true));
            nativeProperties.add("operation", field("string", "Executed native command"));
            nativeProperties.add("result", obj("type", "object"));
            required.addAll(List.of("ok", "operation", "result"));
        } else {
            nativeProperties.add("state", obj("type", "string", "enum", value(List.of("open", "closed", "unavailable", "awaiting_permission", "launch_failed"))));
            nativeProperties.add("session", field("string", "Broker-issued identity for the open session"));
            nativeProperties.add("socket", field("string", "Broker socket path"));
            nativeProperties.add("pid", field("integer", "Owning broker or pending launcher process ID"));
            nativeProperties.add("device_path", field("string", "USB device opened by the native broker"));
            required.add("state");
        }
        JsonObject output = obj("type", "object", "required", List.of("schema", "tool", "engine", "result", "elapsed_ms"), "properties", obj(
                "schema", obj("type", "string", "const", "apk-engine.result/1"), "tool", obj("type", "string", "const", name),
                "engine", obj("type", "string", "const", "brokkr"), "elapsed_ms", obj("type", "integer", "minimum", 0),
                "result", obj("type", "object", "required", required, "properties", nativeProperties)));
        if (nativeSchema.equals("brokkr.result/1")) {
            String operation = name.equals("probe_device") ? "device-probe" : name.equals("flash_firmware") ? "flash" : name.replace('_', '-');
            boolean found = false;
            for (JsonElement branch : RESULT_SCHEMA.getAsJsonArray("oneOf")) {
                JsonObject properties = branch.getAsJsonObject().getAsJsonObject("properties");
                JsonElement constant = properties.getAsJsonObject("operation").get("const");
                if (constant != null && operation.equals(constant.getAsString()) && properties.getAsJsonObject("ok").get("const").getAsBoolean()) {
                    output.getAsJsonObject("properties").add("result", branch.deepCopy());
                    if (RESULT_SCHEMA.has("$defs")) output.add("$defs", RESULT_SCHEMA.get("$defs").deepCopy());
                    found = true;
                    break;
                }
            }
            if (!found) throw new IllegalStateException("Missing native output schema for " + operation);
        }
        return output;
    }

    static void register() {
        for (String name : List.of("parse_pit", "inspect_firmware", "verify_firmware")) {
            String description = switch (name) {
                case "parse_pit" -> "Parse a Samsung PIT partition table with the native Brokkr engine.";
                case "inspect_firmware" -> "Inspect a Samsung firmware TAR or TAR.MD5 archive with the native Brokkr engine.";
                default -> "Verify a Samsung firmware archive and its available checksum with the native Brokkr engine.";
            };
            tool(name, "brokkr", description, false, false, false, outputSchema(name, "brokkr.result/1"), new String[]{"path"},
                    obj("path", field("string", "Input PIT file or firmware archive")), args -> invoke(name.replace('_', '-'), List.of(input(args, "path").toString())));
        }
        tool("map_firmware_to_pit", "brokkr", "Map firmware archive entries to a PIT partition table without writing the device.", false, false, false,
                outputSchema("map_firmware_to_pit", "brokkr.result/1"), new String[]{"pit_path", "firmware_paths"},
                obj("pit_path", field("string", "PIT file"), "firmware_paths", arrayField("Firmware archive paths")), args -> {
                    List<String> arguments = new ArrayList<>(List.of(input(args, "pit_path").toString()));
                    arguments.addAll(inputPaths(args, "firmware_paths"));
                    return invoke("map-firmware-to-pit", arguments);
                });
        JsonObject open = socketProperties();
        open.add("device", field("string", "Termux USB device path, for example /dev/bus/usb/001/002"));
        tool("open_usb_session", "brokkr", "Request Android USB permission and launch the native Brokkr FD broker. A pending permission request returns awaiting_permission.", true, false, true,
                outputSchema("open_usb_session", "brokkr.usb-broker/1"), new String[]{"device"}, open, BrokkrTools::openSession);
        tool("inspect_usb_session", "brokkr", "Read the live native USB broker status; stale metadata cannot establish an open session.", false, false, true,
                outputSchema("inspect_usb_session", "brokkr.usb-broker/1"), new String[]{}, socketProperties(), BrokkrTools::inspectSession);
        JsonObject close = socketProperties();
        close.add("session", field("string", "Session identity returned by the live broker"));
        tool("close_usb_session", "brokkr", "Close the matching USB broker session and its owned device descriptor.", true, false, true,
                outputSchema("close_usb_session", "brokkr.usb-broker/1"), new String[]{"session"}, close, args -> broker(List.of("--stop", string(args, "session"), "--socket", socket(args)), false));
        tool("probe_device", "brokkr", "Start a Samsung download-mode protocol session and read its PIT through the native USB broker; does not flash partitions.", true, false, true,
                outputSchema("probe_device", "brokkr.result/1"), new String[]{}, deviceProperties(), args -> invoke("device-probe", deviceArguments(args)));
        JsonObject flash = deviceProperties();
        flash.add("firmware_paths", arrayField("Firmware archives to flash through the live USB session"));
        flash.add("reboot", field("boolean", "Reboot after flashing, default false"));
        tool("flash_firmware", "brokkr", "Write Samsung firmware partitions through a live native USB broker session. This changes connected device storage.", true, true, true,
                outputSchema("flash_firmware", "brokkr.result/1"), new String[]{"firmware_paths"}, flash, args -> {
                    List<String> arguments = deviceArguments(args);
                    for (String path : inputPaths(args, "firmware_paths")) arguments.addAll(List.of("--ap", path));
                    if (bool(args, "reboot", false)) arguments.add("--reboot");
                    return invoke("flash", arguments);
                });
        for (String name : List.of("probe_device", "flash_firmware")) {
            TOOLS.get(name).schema().add("dependentRequired", obj("usb_interface", List.of("in_endpoint", "out_endpoint"), "in_endpoint", List.of("usb_interface", "out_endpoint"), "out_endpoint", List.of("usb_interface", "in_endpoint")));
        }
    }

    static List<String> inputPaths(JsonObject args, String key) throws IOException {
        JsonArray values = args.getAsJsonArray(key);
        if (values.isEmpty()) throw new IllegalArgumentException(key + " must contain at least one file");
        List<String> paths = new ArrayList<>();
        for (JsonElement value : values) {
            if (!value.isJsonPrimitive() || !value.getAsJsonPrimitive().isString()) throw new IllegalArgumentException(key + " must contain only string paths");
            paths.add(input(obj("path", value), "path").toString());
        }
        return paths;
    }

    static String executable(String environment, String name, String libraryRelative) throws IOException {
        String configured = System.getenv(environment);
        if (configured != null) {
            Path path = Path.of(configured);
            if (!path.isAbsolute() || !Files.isRegularFile(path) || !Files.isExecutable(path)) throw new IOException(environment + " must name an absolute executable file");
            return path.normalize().toString();
        }
        try {
            Path sibling = Files.readSymbolicLink(Path.of("/proc/self/exe")).getParent().resolve(name);
            if (Files.isRegularFile(sibling) && Files.isExecutable(sibling)) return sibling.toString();
        } catch (IOException ignored) {}
        String home = System.getenv("HOME");
        Path path = Path.of(home == null ? System.getProperty("user.home") : home, "engine_library", "brokkr", libraryRelative);
        if (!Files.isRegularFile(path) || !Files.isExecutable(path)) throw new FileNotFoundException("Native companion unavailable; set " + environment + " to its absolute executable path");
        return path.toAbsolutePath().normalize().toString();
    }

    static String socket(JsonObject args) {
        if (args.has("socket")) {
            Path path = Path.of(string(args, "socket"));
            if (!path.isAbsolute()) throw new IllegalArgumentException("socket must be an absolute path");
            return path.normalize().toString();
        }
        String prefix = System.getenv("PREFIX");
        return Path.of(prefix == null ? "/data/data/com.termux/files/usr" : prefix, "tmp", "brokkr-usb", "socket").toString();
    }

    static List<String> deviceArguments(JsonObject args) {
        List<String> arguments = new ArrayList<>(List.of("--socket", socket(args)));
        if (args.has("usb_interface")) arguments.addAll(List.of("--interface", Integer.toString(integer(args, "usb_interface", 0, 0, 255))));
        if (args.has("in_endpoint")) arguments.addAll(List.of("--in", Integer.toString(integer(args, "in_endpoint", 0, 128, 255))));
        if (args.has("out_endpoint")) arguments.addAll(List.of("--out", Integer.toString(integer(args, "out_endpoint", 0, 1, 127))));
        return arguments;
    }

    static Capture capture(InputStream input, Process process) throws IOException {
        ByteArrayOutputStream output = new ByteArrayOutputStream();
        byte[] bytes = new byte[8192];
        int count;
        while ((count = input.read(bytes)) != -1) {
            if (output.size() > OUTPUT_LIMIT - count) {
                process.destroyForcibly();
                throw new IOException("Native companion output exceeds " + OUTPUT_LIMIT + " bytes");
            }
            output.write(bytes, 0, count);
        }
        return new Capture(output.toString(StandardCharsets.UTF_8), output.size());
    }

    static Execution execute(List<String> command) throws Exception {
        ProcessBuilder builder = new ProcessBuilder(command);
        builder.environment().remove("LD_LIBRARY_PATH");
        builder.environment().remove("LD_PRELOAD");
        Process process = builder.start();
        ExecutorService readers = Executors.newFixedThreadPool(2, runnable -> {
            Thread thread = new Thread(runnable, "apk-engine-brokkr-output");
            thread.setDaemon(true);
            return thread;
        });
        try {
            process.getOutputStream().close();
            Future<Capture> stdout = readers.submit(() -> capture(process.getInputStream(), process));
            Future<Capture> stderr = readers.submit(() -> capture(process.getErrorStream(), process));
            int exitCode = process.waitFor();
            return new Execution(exitCode, stdout.get().text(), stderr.get().text());
        } finally {
            if (process.isAlive()) process.destroyForcibly();
            readers.shutdownNow();
            process.getInputStream().close();
            process.getErrorStream().close();
        }
    }

    static JsonObject invoke(String operation, List<String> arguments) throws Exception {
        List<String> command = new ArrayList<>(List.of(executable(ENGINE_ENV, "brokkr-engine", "bin/brokkr-engine"), operation));
        command.addAll(arguments);
        Execution execution = execute(command);
        JsonObject result = parseObject(execution.stdout());
        if (!"brokkr.result/1".equals(string(result, "schema")) || !operation.equals(string(result, "operation")) || !result.has("ok") || !result.get("ok").isJsonPrimitive() || !result.getAsJsonPrimitive("ok").isBoolean()) throw new IOException("Invalid native Brokkr result envelope");
        if (execution.exitCode() != 0 || !result.get("ok").getAsBoolean()) throw new NativeFailure("Brokkr " + operation + " failed with exit " + execution.exitCode() + ": " + result.get("error"), result, execution.exitCode());
        if (!result.has("result") || !result.get("result").isJsonObject()) throw new IOException("Native Brokkr result must be an object");
        return result;
    }

    static JsonObject broker(List<String> arguments, boolean unavailableAllowed) throws Exception {
        List<String> command = new ArrayList<>(List.of(executable(BROKER_ENV, "brokkr-usb-broker", "usb/bin/brokkr-usb-broker")));
        command.addAll(arguments);
        Execution execution = execute(command);
        JsonObject status = parseObject(execution.stdout());
        if (!"brokkr.usb-broker/1".equals(string(status, "schema"))) throw new IOException("Invalid native USB broker envelope");
        if (execution.exitCode() != 0 && !(unavailableAllowed && "unavailable".equals(string(status, "state")))) throw new NativeFailure("USB broker failed with exit " + execution.exitCode() + ": " + status, status, execution.exitCode());
        return status;
    }

    static JsonObject inspectSession(JsonObject args) throws Exception {
        return broker(List.of("--status", "--socket", socket(args)), true);
    }

    static JsonObject openSession(JsonObject args) throws Exception {
        JsonObject current = inspectSession(args);
        if ("open".equals(string(current, "state"))) throw new IOException("A USB broker session is already open at " + socket(args));
        String broker = executable(BROKER_ENV, "brokkr-usb-broker", "usb/bin/brokkr-usb-broker");
        String configured = System.getenv(USB_ENV);
        String prefix = System.getenv("PREFIX");
        Path usb = configured == null ? Path.of(prefix == null ? "/data/data/com.termux/files/usr" : prefix, "bin", "termux-usb") : Path.of(configured);
        if (!usb.isAbsolute() || !Files.isRegularFile(usb) || !Files.isExecutable(usb)) throw new IOException("Termux USB executable unavailable; set " + USB_ENV);
        String socket = socket(args);
        if (!broker.matches("[A-Za-z0-9_./:-]+") || !socket.matches("[A-Za-z0-9_./:-]+")) throw new IllegalArgumentException("Termux USB callback paths must not contain whitespace or shell expansion characters");
        ProcessBuilder builder = new ProcessBuilder(usb.toString(), "-r", "-E", "-e", broker + " --socket " + socket, string(args, "device"));
        builder.environment().put("BROKKR_USB_SOCKET", socket(args));
        builder.redirectOutput(ProcessBuilder.Redirect.DISCARD);
        builder.redirectError(ProcessBuilder.Redirect.DISCARD);
        Process launcher = builder.start();
        launcher.getOutputStream().close();
        for (int i = 0; i < 10; i++) {
            current = inspectSession(args);
            if ("open".equals(string(current, "state"))) return current;
            if (!launcher.isAlive()) return obj("schema", "brokkr.usb-broker/1", "state", "launch_failed", "socket", socket(args), "pid", launcher.pid(), "exit_code", launcher.exitValue());
            Thread.sleep(200);
        }
        return obj("schema", "brokkr.usb-broker/1", "state", "awaiting_permission", "socket", socket(args), "pid", launcher.pid(), "device_path", string(args, "device"));
    }
}
