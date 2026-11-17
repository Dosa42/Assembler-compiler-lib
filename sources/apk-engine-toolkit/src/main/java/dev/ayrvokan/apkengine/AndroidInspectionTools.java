package dev.ayrvokan.apkengine;

import com.google.gson.*;
import com.google.gson.stream.JsonReader;
import com.google.gson.stream.JsonToken;
import dev.ayrvokan.rsm.meminfo.MemInfoParser;
import dev.ayrvokan.rsm.process.ProcessParser;
import java.io.*;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.*;
import java.util.regex.*;
import static dev.ayrvokan.apkengine.Main.*;

final class AndroidInspectionTools {
    static final int OUTPUT_LIMIT = 16 * 1024 * 1024;
    static final int TIMEOUT_SECONDS = 40;
    static final Pattern PACKAGE = Pattern.compile("[A-Za-z_][A-Za-z0-9_]*(?:\\.[A-Za-z_][A-Za-z0-9_]*)*");
    static final Pattern MEMINFO_SECTION = Pattern.compile("(?m)^\\s*\\*\\* MEMINFO in pid \\d+[^\\r\\n]*");
    record Capture(String text, int bytes) {}
    record Acquisition(String text, JsonObject metadata) {}

    static void register() {
        tool("parse_process_snapshot", "rsm-process-parser", "Parse one or more saved dumpsys activity services, activity lru and meminfo outputs.", false, new String[]{},
                obj("services_path", field("string", "UTF-8 dumpsys activity services file"), "lru_path", field("string", "UTF-8 dumpsys activity lru file"), "meminfo_path", field("string", "UTF-8 device-wide dumpsys meminfo file"), "package", field("string", "Optional exact Android package filter")), AndroidInspectionTools::parseProcesses);
        TOOLS.get("parse_process_snapshot").schema().add("anyOf", value(List.of(obj("required", List.of("services_path")), obj("required", List.of("lru_path")), obj("required", List.of("meminfo_path")))));
        tool("parse_meminfo", "rsm-meminfo-parser", "Parse saved detailed dumpsys meminfo output; multiple process sections remain separate.", false, new String[]{"path"},
                obj("path", field("string", "UTF-8 detailed dumpsys meminfo file"), "raw_output", field("boolean", "Include original parser input, default false")), AndroidInspectionTools::parseMemory);
        tool("inspect_android_processes", "rsm-process-parser", "Acquire and parse Android services, LRU processes and device-wide memory through the configured ADB command.", false, new String[]{},
                obj("package", field("string", "Optional exact Android package filter")), AndroidInspectionTools::inspectProcesses);
        tool("inspect_android_meminfo", "rsm-meminfo-parser", "Acquire and parse detailed Android package memory through the configured ADB command.", false, new String[]{"package"},
                obj("package", field("string", "Android package identifier"), "raw_output", field("boolean", "Include original parser input, default false")), AndroidInspectionTools::inspectMemory);
    }

    static String packageName(JsonObject args) {
        if (!args.has("package")) return null;
        String name = string(args, "package");
        if (name.length() > 255 || !PACKAGE.matcher(name).matches()) throw new IllegalArgumentException("Invalid Android package identifier");
        return name;
    }

    static String fileText(JsonObject args, String key) throws Exception {
        if (!args.has(key)) return null;
        Path path = input(args, key);
        try (InputStream stream = Files.newInputStream(path)) { return capture(stream, null).text(); }
    }

    static JsonObject processResult(String services, String lru, String meminfo, String filter) {
        JsonObject result = value(ProcessParser.parse(services, lru, meminfo)).getAsJsonObject();
        if (filter != null) {
            for (String key : List.of("apps", "services", "processes", "bindings")) {
                JsonArray selected = new JsonArray();
                for (JsonElement element : result.getAsJsonArray(key)) {
                    JsonObject item = element.getAsJsonObject();
                    String field = key.equals("bindings") ? "ownerPackageName" : "packageName";
                    JsonElement identity = item.get(field);
                    if (identity != null && !identity.isJsonNull() && filter.equals(identity.getAsString())) selected.add(item);
                }
                result.add(key, selected);
            }
        }
        result.add("package_filter", value(filter));
        result.addProperty("system_ram_scope", "device");
        return result;
    }

    static JsonObject parseProcesses(JsonObject args) throws Exception {
        String filter = packageName(args);
        if (!args.has("services_path") && !args.has("lru_path") && !args.has("meminfo_path")) throw new IllegalArgumentException("At least one of services_path, lru_path or meminfo_path is required");
        return processResult(fileText(args, "services_path"), fileText(args, "lru_path"), fileText(args, "meminfo_path"), filter);
    }

    static JsonObject memorySection(String text, boolean raw) {
        JsonObject result = value(MemInfoParser.parse(text)).getAsJsonObject();
        if (!raw) result.remove("rawOutput");
        return result;
    }

    static JsonObject memoryResult(String text, boolean raw) {
        Matcher matcher = MEMINFO_SECTION.matcher(text);
        List<Integer> positions = new ArrayList<>();
        while (matcher.find()) positions.add(matcher.start());
        if (positions.size() < 2) return memorySection(text, raw);
        String header = text.substring(0, positions.get(0));
        JsonArray processes = new JsonArray();
        for (int i = 0; i < positions.size(); i++) {
            int end = i + 1 < positions.size() ? positions.get(i + 1) : text.length();
            processes.add(memorySection(header + text.substring(positions.get(i), end), raw));
        }
        return obj("schema", "apk-engine.meminfo-batch/1", "processCount", processes.size(), "processes", processes);
    }

    static JsonObject parseMemory(JsonObject args) throws Exception {
        return memoryResult(fileText(args, "path"), bool(args, "raw_output", false));
    }

    static List<String> commandPrefix() throws Exception {
        String configured = System.getenv("APK_ENGINE_ADB_COMMAND_JSON");
        if (configured == null) {
            String home = System.getenv("HOME");
            if (home == null || home.isBlank()) home = System.getProperty("user.home");
            Path homePath = Path.of(home).toAbsolutePath().normalize();
            String helper = homePath.resolve(".local/bin/phone-adb").toString();
            String prefix = System.getenv("PREFIX");
            if (prefix != null && prefix.endsWith("/com.termux/files/usr")) {
                Path python = Path.of(prefix, "bin", "python3").toAbsolutePath().normalize();
                if (Files.isExecutable(python)) return List.of(python.toString(), helper);
            }
            if (homePath.getParent() != null && homePath.getParent().toString().endsWith("/com.termux/files")) {
                Path python = homePath.getParent().resolve("usr/bin/python3");
                if (Files.isExecutable(python)) return List.of(python.toString(), helper);
            }
            return List.of(helper);
        }
        JsonReader reader = new JsonReader(new StringReader(configured));
        reader.setStrictness(Strictness.STRICT);
        JsonElement parsed = JsonParser.parseReader(reader);
        if (reader.peek() != JsonToken.END_DOCUMENT || !parsed.isJsonArray() || parsed.getAsJsonArray().isEmpty()) throw new IllegalArgumentException("APK_ENGINE_ADB_COMMAND_JSON must be a nonempty JSON string array");
        List<String> prefix = new ArrayList<>();
        for (JsonElement element : parsed.getAsJsonArray()) {
            if (!element.isJsonPrimitive() || !element.getAsJsonPrimitive().isString() || element.getAsString().isEmpty() || element.getAsString().indexOf('\0') >= 0) throw new IllegalArgumentException("APK_ENGINE_ADB_COMMAND_JSON contains an invalid argument");
            prefix.add(element.getAsString());
        }
        if (!Path.of(prefix.get(0)).isAbsolute()) throw new IllegalArgumentException("ADB executable must be an absolute path");
        return prefix;
    }

    static Capture capture(InputStream stream, Process process) throws IOException {
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        byte[] buffer = new byte[8192];
        int count;
        while ((count = stream.read(buffer)) != -1) {
            if (bytes.size() > OUTPUT_LIMIT - count) {
                if (process != null) process.destroyForcibly();
                throw new IOException("Output exceeds " + OUTPUT_LIMIT + " byte limit");
            }
            bytes.write(buffer, 0, count);
        }
        return new Capture(StandardCharsets.UTF_8.newDecoder().decode(ByteBuffer.wrap(bytes.toByteArray())).toString(), bytes.size());
    }

    static Capture completed(Future<Capture> future) throws Exception {
        try { return future.get(5, TimeUnit.SECONDS); }
        catch (ExecutionException error) {
            if (error.getCause() instanceof Exception cause) throw cause;
            throw new IOException("Failed to read ADB output", error.getCause());
        }
    }

    static Acquisition acquire(String... arguments) throws Exception {
        List<String> command = new ArrayList<>(commandPrefix());
        command.addAll(List.of("shell", "dumpsys", "-t", "30"));
        command.addAll(List.of(arguments));
        long started = System.nanoTime();
        Process process = new ProcessBuilder(command).start();
        ExecutorService readers = Executors.newFixedThreadPool(2, runnable -> {
            Thread thread = new Thread(runnable, "apk-engine-adb-output");
            thread.setDaemon(true);
            return thread;
        });
        try {
            process.getOutputStream().close();
            Future<Capture> stdout = readers.submit(() -> capture(process.getInputStream(), process));
            Future<Capture> stderr = readers.submit(() -> capture(process.getErrorStream(), process));
            if (!process.waitFor(TIMEOUT_SECONDS, TimeUnit.SECONDS)) throw new IOException("ADB command timed out after " + TIMEOUT_SECONDS + " seconds");
            Capture output = completed(stdout), errors = completed(stderr);
            if (process.exitValue() != 0) throw new IOException("ADB command failed with exit " + process.exitValue() + ": " + errors.text());
            if (output.text().isBlank()) throw new IOException("ADB command returned empty output");
            if (output.text().contains("Permission Denial:") || output.text().contains("Can't find service:") || output.text().contains("No process found for:")) throw new IOException("ADB dumpsys failed: " + output.text());
            return new Acquisition(output.text(), obj("command", command, "exit_code", process.exitValue(), "stdout_bytes", output.bytes(), "stderr_bytes", errors.bytes(), "stderr", errors.text(), "elapsed_ms", (System.nanoTime() - started) / 1000000));
        } finally {
            if (process.isAlive()) {
                process.destroyForcibly();
                process.waitFor(5, TimeUnit.SECONDS);
            }
            process.getInputStream().close();
            process.getErrorStream().close();
            readers.shutdownNow();
        }
    }

    static JsonObject acquisition(Acquisition... commands) {
        JsonArray records = new JsonArray();
        for (Acquisition command : commands) records.add(command.metadata());
        return obj("method", "adb", "commands", records, "timeout_seconds", TIMEOUT_SECONDS, "output_limit_bytes", OUTPUT_LIMIT, "sequential", true);
    }

    static JsonObject inspectProcesses(JsonObject args) throws Exception {
        String filter = packageName(args);
        Acquisition services = acquire("activity", "services"), lru = acquire("activity", "lru"), meminfo = acquire("meminfo");
        JsonObject result = processResult(services.text(), lru.text(), meminfo.text(), filter);
        result.add("acquisition", acquisition(services, lru, meminfo));
        return result;
    }

    static JsonObject inspectMemory(JsonObject args) throws Exception {
        String name = packageName(args);
        Acquisition memory = acquire("meminfo", name);
        JsonObject result = memoryResult(memory.text(), bool(args, "raw_output", false));
        result.add("acquisition", acquisition(memory));
        return result;
    }
}
