package dev.ayrvokan.apkengine;

import com.google.gson.*;
import org.jf.baksmali.Baksmali;
import org.jf.baksmali.BaksmaliOptions;
import org.jf.dexlib2.DexFileFactory;
import org.jf.dexlib2.Opcodes;
import org.jf.dexlib2.iface.*;
import org.jf.smali.Smali;
import org.jf.smali.SmaliOptions;
import java.io.*;
import java.nio.file.*;
import java.util.*;
import static dev.ayrvokan.apkengine.Main.*;

final class DexTools {
    static void register() {
        tool("list_dex_classes", "dexlib2", "List classes defined in a DEX or APK, with method and field counts.", false, new String[]{"path"},
                obj("path", field("string", "DEX or APK path"), "entry", field("string", "Optional exact DEX entry"), "prefix", field("string", "Class descriptor prefix"), "offset", field("integer", "Page offset"), "limit", field("integer", "Page size 1..1000, default100"), "api", field("integer", "Android API, default28")), DexTools::classes);
        tool("disassemble_dex", "baksmali", "Disassemble a DEX or selected APK DEX entry into smali files.", true, new String[]{"path", "output_dir"},
                obj("path", field("string", "DEX or APK path"), "entry", field("string", "DEX entry, required for an APK with multiple DEX files"), "output_dir", field("string", "Empty output directory"), "api", field("integer", "Android API, default28")), DexTools::disassemble);
        tool("assemble_dex", "smali", "Assemble a directory of smali source into a DEX file.", true, new String[]{"source_dir", "output"},
                obj("source_dir", field("string", "Smali source directory"), "output", field("string", "Output DEX path"), "api", field("integer", "Android API, default28"), "overwrite", field("boolean", "Replace existing output, defaultfalse")), DexTools::assemble);
    }

    static MultiDexContainer<? extends DexFile> container(JsonObject args) throws Exception {
        return DexFileFactory.loadDexContainer(input(args, "path").toFile(), Opcodes.forApi(integer(args, "api", 28, 1, 37)));
    }

    static DexFile selected(JsonObject args) throws Exception {
        var container = container(args);
        List<String> names = container.getDexEntryNames();
        String name;
        if (args.has("entry")) name = string(args, "entry");
        else if (names.size() == 1) name = names.get(0);
        else throw new IllegalArgumentException("Select entry from " + names);
        var entry = container.getEntry(name);
        if (entry == null) throw new FileNotFoundException("DEX entry not found: " + name);
        return entry.getDexFile();
    }

    static int count(Iterable<?> values) {
        int count = 0;
        for (Object value : values) count++;
        return count;
    }

    static JsonObject classes(JsonObject args) throws Exception {
        var container = container(args);
        List<String> names = container.getDexEntryNames();
        if (args.has("entry")) {
            String entry = string(args, "entry");
            if (!names.contains(entry)) throw new FileNotFoundException("DEX entry not found: " + entry);
            names = List.of(entry);
        }
        String prefix = args.has("prefix") ? string(args, "prefix") : "";
        int offset = integer(args, "offset", 0, 0, Integer.MAX_VALUE), limit = integer(args, "limit", 100, 1, 1000);
        List<JsonObject> items = new ArrayList<>();
        for (String name : names) {
            for (ClassDef cls : container.getEntry(name).getDexFile().getClasses()) {
                if (cls.getType().startsWith(prefix)) items.add(obj("entry", name, "type", cls.getType(), "superclass", cls.getSuperclass(),
                        "access_flags", cls.getAccessFlags(), "methods", count(cls.getMethods()), "fields", count(cls.getFields())));
            }
        }
        items.sort(Comparator.comparing(item -> item.get("entry").getAsString() + item.get("type").getAsString()));
        int end = (int)Math.min(items.size(), (long)offset + limit);
        return obj("path", input(args, "path"), "dex_entries", names, "total", items.size(), "classes", items.subList(Math.min(offset, items.size()), end), "next_offset", end < items.size() ? end : null);
    }

    static JsonObject disassemble(JsonObject args) throws Exception {
        DexFile dex = selected(args);
        Path output = destination(args);
        BaksmaliOptions options = new BaksmaliOptions();
        options.apiLevel = integer(args, "api", 28, 1, 37);
        if (!Baksmali.disassembleDexFile(dex, output.toFile(), 2, options)) throw new IOException("Baksmali disassembly failed");
        return obj("output_dir", output, "classes", dex.getClasses().size());
    }

    static JsonObject assemble(JsonObject args) throws Exception {
        Path source = Path.of(string(args, "source_dir")).toAbsolutePath().normalize();
        if (!Files.isDirectory(source)) throw new IOException("Smali source directory does not exist: " + source);
        Path target = output(args, "output");
        int api = integer(args, "api", 28, 1, 37);
        return writeArtifact(args, target, temporary -> {
            SmaliOptions options = new SmaliOptions();
            options.apiLevel = api;
            options.jobs = 2;
            options.outputDexFile = temporary.toString();
            if (!Smali.assemble(options, source.toString())) throw new IOException("Smali assembly failed");
        });
    }
}
