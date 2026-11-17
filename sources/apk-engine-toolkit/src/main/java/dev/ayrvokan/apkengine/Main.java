package dev.ayrvokan.apkengine;

import com.google.gson.*;
import com.google.gson.stream.JsonReader;
import com.google.gson.stream.JsonToken;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.security.MessageDigest;
import java.util.*;

public final class Main {
    public static final String VERSION = "0.2.0";
    static final PrintWriter OUT = new PrintWriter(System.out, true, StandardCharsets.UTF_8);
    static final Map<String, Tool> TOOLS = new LinkedHashMap<>();
    interface Handler { JsonObject run(JsonObject args) throws Exception; }
    interface FileWriter { void write(Path path) throws Exception; }
    record Tool(String name, String engine, String description, JsonObject schema, boolean writes, Handler handler) {}

    static {
        ArchiveTools.register();
        DexTools.register();
        XmlTools.register();
        SigningTools.register();
        AndroidInspectionTools.register();
    }

    public static JsonObject obj(Object... pairs) {
        JsonObject out = new JsonObject();
        for (int i = 0; i < pairs.length; i += 2) out.add((String)pairs[i], value(pairs[i + 1]));
        return out;
    }

    public static JsonElement value(Object v) {
        if (v == null) return JsonNull.INSTANCE;
        if (v instanceof JsonElement e) return e;
        if (v instanceof Boolean b) return new JsonPrimitive(b);
        if (v instanceof Number n) return new JsonPrimitive(n);
        if (v instanceof Path || v instanceof File) return new JsonPrimitive(v.toString());
        if (v instanceof Map<?, ?> entries) {
            JsonObject out = new JsonObject();
            for (var entry : entries.entrySet()) out.add(String.valueOf(entry.getKey()), value(entry.getValue()));
            return out;
        }
        if (v instanceof Iterable<?> items) {
            JsonArray a = new JsonArray();
            for (Object item : items) a.add(value(item));
            return a;
        }
        return new JsonPrimitive(v.toString());
    }

    static JsonObject field(String type, String description) {
        return obj("type", type, "description", description);
    }

    static void tool(String name, String engine, String description, boolean writes, String[] required, JsonObject properties, Handler handler) {
        JsonArray req = new JsonArray();
        for (String key : required) req.add(key);
        TOOLS.put(name, new Tool(name, engine, description,
                obj("type", "object", "properties", properties, "required", req, "additionalProperties", false), writes, handler));
    }

    static JsonArray toolList() {
        JsonArray list = new JsonArray();
        for (Tool t : TOOLS.values()) list.add(obj("name", t.name(), "description", t.description(), "inputSchema", t.schema(),
                "annotations", obj("readOnlyHint", !t.writes(), "destructiveHint", t.writes(), "openWorldHint", false)));
        return list;
    }

    static String string(JsonObject a, String key) {
        if (!a.has(key) || !a.get(key).isJsonPrimitive() || !a.getAsJsonPrimitive(key).isString()) throw new IllegalArgumentException("Expected string: " + key);
        return a.get(key).getAsString();
    }

    static int integer(JsonObject a, String key, int fallback, int min, int max) {
        if (!a.has(key)) return fallback;
        try {
            int n = a.get(key).getAsBigDecimal().intValueExact();
            if (n < min || n > max) throw new ArithmeticException();
            return n;
        } catch (Exception e) { throw new IllegalArgumentException(key + " must be an integer in " + min + ".." + max); }
    }

    static boolean bool(JsonObject a, String key, boolean fallback) {
        return a.has(key) ? a.get(key).getAsBoolean() : fallback;
    }

    static Path input(JsonObject a, String key) throws IOException {
        Path path = Path.of(string(a, key)).toAbsolutePath().normalize();
        if (!Files.isRegularFile(path)) throw new FileNotFoundException("Not a file: " + path);
        return path;
    }

    static Path output(JsonObject a, String key) throws IOException {
        Path path = Path.of(string(a, key)).toAbsolutePath().normalize();
        if (Files.exists(path) && !bool(a, "overwrite", false)) throw new FileAlreadyExistsException(path.toString());
        if (Files.isDirectory(path)) throw new IOException("Output is a directory: " + path);
        Files.createDirectories(path.getParent());
        return path;
    }

    static Path destination(JsonObject a) throws IOException {
        Path path = Path.of(string(a, "output_dir")).toAbsolutePath().normalize();
        if (Files.exists(path)) {
            if (!Files.isDirectory(path)) throw new IOException("Not a directory: " + path);
            try (var files = Files.list(path)) {
                if (files.findAny().isPresent()) throw new IOException("Output directory must be empty: " + path);
            }
        }
        Files.createDirectories(path);
        return path;
    }

    static void different(Path source, Path target) throws IOException {
        if (source.equals(target) || (Files.exists(target) && Files.isSameFile(source, target))) throw new IOException("Input and output must be distinct");
    }

    static String sha256(Path path) throws Exception {
        MessageDigest md = MessageDigest.getInstance("SHA-256");
        try (InputStream in = Files.newInputStream(path)) {
            byte[] buffer = new byte[65536];
            int read;
            while ((read = in.read(buffer)) != -1) md.update(buffer, 0, read);
        }
        return HexFormat.of().formatHex(md.digest());
    }

    static JsonObject artifact(Path path) throws Exception {
        return obj("path", path.toString(), "bytes", Files.size(path), "sha256", sha256(path));
    }

    static JsonObject writeArtifact(JsonObject args, Path target, FileWriter writer) throws Exception {
        Path temporary = Files.createTempFile(target.getParent(), ".apk-engine-", ".part");
        Files.delete(temporary);
        try {
            writer.write(temporary);
            if (bool(args, "overwrite", false)) Files.move(temporary, target, StandardCopyOption.REPLACE_EXISTING);
            else Files.move(temporary, target);
            return artifact(target);
        } finally { Files.deleteIfExists(temporary); }
    }

    static void validate(Tool tool, JsonObject args) {
        JsonObject properties = tool.schema().getAsJsonObject("properties");
        for (String key : args.keySet()) {
            if (!properties.has(key)) throw new IllegalArgumentException("Unknown argument: " + key);
            JsonElement v = args.get(key);
            String type = properties.getAsJsonObject(key).get("type").getAsString();
            boolean ok = switch (type) {
                case "string" -> v.isJsonPrimitive() && v.getAsJsonPrimitive().isString();
                case "integer" -> v.isJsonPrimitive() && v.getAsJsonPrimitive().isNumber() && v.getAsBigDecimal().stripTrailingZeros().scale() <= 0;
                case "boolean" -> v.isJsonPrimitive() && v.getAsJsonPrimitive().isBoolean();
                case "array" -> v.isJsonArray();
                default -> false;
            };
            if (!ok) throw new IllegalArgumentException("Invalid " + type + " argument: " + key);
        }
        for (JsonElement key : tool.schema().getAsJsonArray("required")) {
            if (!args.has(key.getAsString())) throw new IllegalArgumentException("Missing argument: " + key.getAsString());
        }
    }

    static JsonObject call(String name, JsonObject args) throws Exception {
        Tool tool = TOOLS.get(name.replace('-', '_'));
        if (tool == null) throw new IllegalArgumentException("Unknown tool: " + name);
        validate(tool, args);
        long start = System.nanoTime();
        JsonObject result = tool.handler().run(args);
        return obj("schema", "apk-engine.result/1", "tool", tool.name(), "engine", tool.engine(),
                "result", result, "elapsed_ms", (System.nanoTime() - start) / 1000000);
    }

    static JsonObject error(Exception e) {
        return obj("type", e.getClass().getSimpleName(), "message", e.getMessage() == null ? e.toString() : e.getMessage());
    }

    static JsonObject parseObject(String text) {
        try {
            JsonReader reader = new JsonReader(new StringReader(text));
            reader.setStrictness(Strictness.STRICT);
            JsonElement value = JsonParser.parseReader(reader);
            if (reader.peek() != JsonToken.END_DOCUMENT) throw new IllegalArgumentException("Trailing JSON data");
            if (!value.isJsonObject()) throw new IllegalArgumentException("Expected a JSON object");
            return value.getAsJsonObject();
        } catch (IOException e) { throw new IllegalArgumentException("Invalid JSON", e); }
    }

    static JsonObject readArgs(String text) throws IOException {
        if (text.equals("-")) return parseObject(new String(System.in.readAllBytes(), StandardCharsets.UTF_8));
        if (text.stripLeading().startsWith("{")) return parseObject(text);
        return parseObject(Files.readString(Path.of(text), StandardCharsets.UTF_8));
    }

    static JsonObject rpcError(JsonElement id, int code, String message) {
        return obj("jsonrpc", "2.0", "id", id, "error", obj("code", code, "message", message));
    }

    static void mcp() throws IOException {
        boolean initialized = false;
        boolean ready = false;
        BufferedReader reader = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
        String line;
        while ((line = reader.readLine()) != null) {
            JsonObject request;
            try { request = parseObject(line); }
            catch (Exception e) { OUT.println(rpcError(JsonNull.INSTANCE, -32700, "Parse error")); continue; }
            JsonElement id = request.has("id") ? request.get("id") : null;
            if (!request.has("method")) {
                if (request.has("result") || request.has("error")) continue;
                OUT.println(rpcError(id, -32600, "Invalid Request"));
                continue;
            }
            String method;
            try {
                if (!"2.0".equals(string(request, "jsonrpc"))) throw new IllegalArgumentException();
                method = string(request, "method");
                if (id != null && (!id.isJsonPrimitive() || id.getAsJsonPrimitive().isBoolean())) throw new IllegalArgumentException();
            } catch (Exception e) { OUT.println(rpcError(id, -32600, "Invalid Request")); continue; }
            if (id == null) {
                if (method.equals("notifications/initialized") && initialized) ready = true;
                continue;
            }
            JsonObject result;
            try {
                JsonObject params = request.has("params") ? request.getAsJsonObject("params") : new JsonObject();
                switch (method) {
                    case "initialize" -> {
                        if (initialized) throw new IllegalArgumentException("Already initialized");
                        String version = string(params, "protocolVersion");
                        if (!Set.of("2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25").contains(version)) version = "2025-11-25";
                        initialized = true;
                        result = obj("protocolVersion", version, "capabilities", obj("tools", obj("listChanged", false)),
                                "serverInfo", obj("name", "apk-engine", "version", VERSION), "instructions", "Local APK engine toolkit. CLI and MCP share the same tools and schemas. Output paths are explicit.");
                    }
                    case "ping" -> result = new JsonObject();
                    case "tools/list" -> {
                        if (!ready) throw new IllegalArgumentException("Initialize the session first");
                        if (params.has("cursor")) throw new IllegalArgumentException("This catalog has no pagination cursor");
                        result = obj("tools", toolList());
                    }
                    case "tools/call" -> {
                        if (!ready) throw new IllegalArgumentException("Initialize the session first");
                        String name = string(params, "name");
                        if (!TOOLS.containsKey(name)) throw new IllegalArgumentException("Unknown tool: " + name);
                        JsonObject args = params.has("arguments") ? params.getAsJsonObject("arguments") : new JsonObject();
                        validate(TOOLS.get(name), args);
                        try {
                            JsonObject data = call(name, args);
                            result = obj("content", List.of(obj("type", "text", "text", data.toString())), "structuredContent", data, "isError", false);
                        } catch (Exception e) {
                            JsonObject data = obj("error", error(e));
                            result = obj("content", List.of(obj("type", "text", "text", data.toString())), "structuredContent", data, "isError", true);
                        }
                    }
                    default -> { OUT.println(rpcError(id, -32601, "Method not found")); continue; }
                }
                OUT.println(obj("jsonrpc", "2.0", "id", id, "result", result));
            } catch (Exception e) { OUT.println(rpcError(id, -32602, e.getMessage() == null ? "Invalid params" : e.getMessage())); }
        }
    }

    public static void main(String[] args) {
        System.setOut(System.err);
        try {
            if (args.length == 0 || args[0].equals("--help") || args[0].equals("help")) {
                OUT.println("apk-engine " + VERSION + " — APK engines for terminal and MCP\nUsage: apk-engine <command> '<json>' | <input.json> | -\n       apk-engine <command> --schema\n       apk-engine --mcp\n       apk-engine tools\n");
                for (Tool t : TOOLS.values()) OUT.println("  " + t.name().replace('_', '-') + "  " + t.description());
                return;
            }
            if (args[0].equals("--mcp") || args[0].equals("mcp")) { mcp(); return; }
            if (args[0].equals("--version")) { OUT.println(VERSION); return; }
            if (args[0].equals("tools") || args[0].equals("--llms")) { OUT.println(obj("name", "apk-engine", "version", VERSION, "tools", toolList())); return; }
            String name = args[0].replace('-', '_');
            Tool tool = TOOLS.get(name);
            if (tool == null) throw new IllegalArgumentException("Unknown command: " + args[0]);
            if (args.length == 2 && args[1].equals("--schema")) { OUT.println(tool.schema()); return; }
            if (args.length > 2) throw new IllegalArgumentException("Pass one JSON argument, JSON file, or - for stdin");
            OUT.println(call(name, args.length == 2 ? readArgs(args[1]) : new JsonObject()));
        } catch (Exception e) { OUT.println(obj("error", error(e))); System.exit(1); }
    }
}
