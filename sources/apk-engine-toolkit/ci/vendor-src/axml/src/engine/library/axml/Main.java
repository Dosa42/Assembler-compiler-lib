package engine.library.axml;

import com.apk.axml.ResourceTableParser;
import com.apk.axml.aXMLDecoder;
import com.apk.axml.aXMLEncoder;
import com.apk.axml.serializables.ResEntry;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

public final class Main {
    private static String quote(String value) {
        if (value == null) return "null";
        StringBuilder out = new StringBuilder("\"");
        for (int i = 0; i < value.length(); i++) {
            char c = value.charAt(i);
            if (c == '\\' || c == '"') out.append('\\').append(c);
            else if (c < 32) out.append(String.format("\\u%04x", (int) c));
            else out.append(c);
        }
        return out.append('"').toString();
    }

    public static String resourceJson(InputStream input) throws Exception {
        List<ResEntry> entries = new ResourceTableParser(input).parse();
        StringBuilder out = new StringBuilder("[");
        for (ResEntry entry : entries) {
            if (out.length() > 1) out.append(',');
            out.append("{\"id\":").append(Integer.toUnsignedLong(entry.getResourceId()));
            out.append(",\"name\":").append(quote(entry.getName()));
            out.append(",\"value\":").append(quote(entry.getValue())).append('}');
        }
        return out.append(']').toString();
    }

    public static void main(String[] args) throws Exception {
        if (args.length != 3) throw new IllegalArgumentException("encode|decode|resources input output");
        Path input = Path.of(args[1]);
        Path output = Path.of(args[2]);
        switch (args[0]) {
            case "encode":
                Files.write(output, new aXMLEncoder().encode(Files.readString(input, StandardCharsets.UTF_8)));
                break;
            case "decode":
                try (InputStream stream = Files.newInputStream(input)) {
                    Files.writeString(output, new aXMLDecoder(stream).decodeAsString(), StandardCharsets.UTF_8);
                }
                break;
            case "resources":
                try (InputStream stream = Files.newInputStream(input)) {
                    Files.writeString(output, resourceJson(stream), StandardCharsets.UTF_8);
                }
                break;
            default:
                throw new IllegalArgumentException("Unknown action: " + args[0]);
        }
    }
}
