package dev.ayrvokan.apkengine;

import com.google.gson.*;
import com.apk.axml.aXMLDecoder;
import com.apk.axml.aXMLEncoder;
import com.apk.axml.ResourceTableParser;
import net.lingala.zip4j.ZipFile;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import static dev.ayrvokan.apkengine.Main.*;

final class XmlTools {
    static void register() {
        tool("decode_xml", "axml", "Decode Android binary XML from a raw file or an APK entry.", false, new String[]{"path"},
                obj("path", field("string", "Binary XML file or APK"), "entry", field("string", "APK entry, for example AndroidManifest.xml")), XmlTools::decode);
        tool("encode_xml", "axml", "Encode an Android XML document as binary XML.", true, new String[]{"path", "output"},
                obj("path", field("string", "UTF-8 XML input"), "output", field("string", "Binary XML output"), "overwrite", field("boolean", "Replace existing output, default false")), XmlTools::encode);
        tool("inspect_resources", "axml", "Read a page of Android resource-table entries from ARSC or an APK entry.", false, new String[]{"path"},
                obj("path", field("string", "resources.arsc file or APK"), "entry", field("string", "APK entry, usually resources.arsc"), "offset", field("integer", "Page offset"), "limit", field("integer", "Page size1..1000, default100")), XmlTools::resources);
    }

    static byte[] bytes(JsonObject args) throws Exception {
        Path path = input(args, "path");
        if (!args.has("entry")) return Files.readAllBytes(path);
        try (ZipFile zip = new ZipFile(path.toFile())) {
            var header = zip.getFileHeader(string(args, "entry"));
            if (header == null) throw new FileNotFoundException("APK entry not found: " + string(args, "entry"));
            try (InputStream in = zip.getInputStream(header)) { return in.readAllBytes(); }
        }
    }

    static JsonObject decode(JsonObject args) throws Exception {
        String xml = new aXMLDecoder(new ByteArrayInputStream(bytes(args))).decodeAsString();
        return obj("path", input(args, "path"), "xml", xml);
    }

    static JsonObject encode(JsonObject args) throws Exception {
        Path source = input(args, "path"), target = output(args, "output");
        different(source, target);
        byte[] encoded = new aXMLEncoder().encode(Files.readString(source, StandardCharsets.UTF_8));
        return writeArtifact(args, target, temporary -> Files.write(temporary, encoded));
    }

    static JsonObject resources(JsonObject args) throws Exception {
        int offset = integer(args, "offset", 0, 0, Integer.MAX_VALUE), limit = integer(args, "limit", 100, 1, 1000);
        var resources = new ResourceTableParser(new ByteArrayInputStream(bytes(args))).parse();
        int end = (int)Math.min(resources.size(), (long)offset + limit);
        JsonArray entries = new JsonArray();
        for (int i = offset; i < end; i++) {
            var item = resources.get(i);
            entries.add(obj("id", String.format("0x%08x", item.getResourceId()), "name", item.getName(), "value", item.getValue(), "attribute", item.getResAttr()));
        }
        return obj("path", input(args, "path"), "total", resources.size(), "offset", offset, "entries", entries, "next_offset", end < resources.size() ? end : null,
                "scope", "Resource IDs/names and values exposed by aXML; complex bags and all locale variants are not fully resolved.");
    }
}
