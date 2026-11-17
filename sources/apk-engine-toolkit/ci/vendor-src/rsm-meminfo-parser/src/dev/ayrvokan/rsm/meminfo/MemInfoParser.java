package dev.ayrvokan.rsm.meminfo;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

public final class MemInfoParser {
    private static final String[] CATEGORY = {"pssTotal", "privateDirty", "privateClean", "swapPssDirty", "rssTotal", "heapSize", "heapAlloc", "heapFree"};
    private static final String[] SUMMARY = {"javaHeapPss", "javaHeapRss", "nativeHeapPss", "nativeHeapRss", "codePss", "codeRss", "stackPss", "stackRss", "graphicsPss", "graphicsRss", "privateOther", "system", "unknownRss", "totalPss", "totalRss", "totalSwapPss"};
    private static final String[] OBJECTS = {"views", "viewRootImpl", "appContexts", "activities", "assets", "assetManagers", "localBinders", "proxyBinders", "parcelMemory", "parcelCount", "deathRecipients", "webViews"};
    private static final String[] NATIVE = {"bitmapCount", "bitmapTotalKb", "otherMallocedCount", "otherMallocedKb", "otherNonMallocedCount", "otherNonMallocedKb"};
    private static final Pattern HEADER = Pattern.compile("MEMINFO\\s+in\\s+pid\\s+(\\d+)(?:\\s*\\[([^\\]]+)\\])?", Pattern.CASE_INSENSITIVE);
    private static final Pattern PID = Pattern.compile("\\bpid\\s+(\\d+)", Pattern.CASE_INSENSITIVE);
    private static final Pattern CLOCK = Pattern.compile("\\b(Uptime|Realtime):\\s*(\\S+)", Pattern.CASE_INSENSITIVE);
    private static final Pattern TOKEN = Pattern.compile("\\S+");
    private static final Pattern ROW = Pattern.compile("^\\s*(.*?)\\s+([+-]?\\d[\\d,]*|N/A|n/a|--|-)(?=\\s|$)");
    private static final Pattern PAIR = Pattern.compile("([\\p{L}][\\p{L}\\p{N} _/()\\-]*?):\\s*(\\S+)");
    private static final Pattern SUMMARY_HEADER = Pattern.compile("\\b(Pss|Rss)\\s*\\((?:[kK][bB]|KiB)\\)", Pattern.CASE_INSENSITIVE);
    private static final Pattern TOTAL_PAIR = Pattern.compile("\\b(TOTAL\\s+SWAP\\s+PSS|TOTAL\\s+PSS|TOTAL\\s+RSS|SWAP\\s+PSS|TOTAL\\s+SWAP|TOTAL):\\s*(\\S+)", Pattern.CASE_INSENSITIVE);

    private MemInfoParser() {
    }

    public static Map<String, Object> parse(String text) {
        if (text != null) {
            Matcher headers = HEADER.matcher(text);
            int count = 0;
            while (headers.find()) {
                if (++count > 1) throw new IllegalArgumentException("Multiple MEMINFO process blocks; split input into one block per process");
            }
        }
        return new State(text).parse();
    }

    private record Cell(String text, int end) {
    }

    private record Column(String key, int end) {
    }

    private static List<Cell> tokens(String line) {
        List<Cell> cells = new ArrayList<>();
        Matcher matcher = TOKEN.matcher(line);
        while (matcher.find()) cells.add(new Cell(matcher.group(), matcher.end()));
        return cells;
    }

    private static Map<String, Object> empty(String[] names) {
        Map<String, Object> result = new LinkedHashMap<>();
        for (String name : names) result.put(name, null);
        return result;
    }

    private static String normalized(String text) {
        return text.toLowerCase(Locale.ROOT).replaceAll("[^a-z0-9]", "");
    }

    private static String columnKey(String upper, String lower) {
        return switch (normalized(upper + lower)) {
            case "psstotal", "pss" -> "pssTotal";
            case "pssclean" -> "pssClean";
            case "privatedirty" -> "privateDirty";
            case "privateclean" -> "privateClean";
            case "shareddirty" -> "sharedDirty";
            case "sharedclean" -> "sharedClean";
            case "swappssdirty", "swappss", "swapps" -> "swapPssDirty";
            case "swappeddirty", "swapped" -> "swappedDirty";
            case "rsstotal", "rss" -> "rssTotal";
            case "heapsize" -> "heapSize";
            case "heapalloc" -> "heapAlloc";
            case "heapfree" -> "heapFree";
            default -> upper + " " + lower;
        };
    }

    private static final class State {
        private final String raw;
        private final Map<String, Object> result = new LinkedHashMap<>();
        private final List<Map<String, Object>> diagnostics = new ArrayList<>();
        private final List<Map<String, Object>> categories = new ArrayList<>();
        private final List<Column> columns = new ArrayList<>();
        private List<Cell> header;
        private Map<String, Object> summary;
        private Map<String, Object> objects;
        private Map<String, Object> allocations;
        private final List<Map<String, Object>> summaryRows = new ArrayList<>();
        private final List<Map<String, Object>> allocationRows = new ArrayList<>();
        private int section;
        private int lineNumber;
        private int pssEnd = -1;
        private int rssEnd = -1;
        private boolean assumedColumns;

        private State(String text) {
            raw = text == null ? "" : text;
            result.put("schema", "ayrvokan.rsm.meminfo/1");
            result.put("status", "unrecognized");
            result.put("packageName", null);
            result.put("processName", null);
            result.put("pid", null);
            result.put("uptimeMs", null);
            result.put("realtimeMs", null);
            result.put("rawOutput", raw);
            result.put("categories", categories);
            result.put("total", null);
            result.put("appSummary", null);
            result.put("objects", null);
            result.put("nativeAllocations", null);
            result.put("units", Map.of("categories", "KiB", "total", "KiB", "appSummary", "KiB", "objects", "count except parcelMemory=KiB", "nativeAllocations", "counts=count; *Kb=KiB", "uptimeMs", "milliseconds", "realtimeMs", "milliseconds"));
            result.put("diagnostics", diagnostics);
            if (text == null) diagnostic("null_input", "warning", "Input text is null", null);
        }

        private Map<String, Object> parse() {
            for (String original : raw.split("\\R", -1)) {
                lineNumber++;
                String line = original.replace("\t", "        ");
                String trimmed = line.trim();
                if (trimmed.isEmpty() || trimmed.matches("[-=\\s]+")) continue;
                Matcher clock = CLOCK.matcher(line);
                boolean clockLine = false;
                while (clock.find()) {
                    result.put(clock.group(1).equalsIgnoreCase("Uptime") ? "uptimeMs" : "realtimeMs", number(clock.group(2), original));
                    clockLine = true;
                }
                if (clockLine) continue;
                Matcher process = HEADER.matcher(line);
                if (process.find()) {
                    result.put("pid", number(process.group(1), original));
                    String name = process.group(2);
                    result.put("processName", name);
                    if (name != null) {
                        String packageName = name.split(":", 2)[0];
                        if (packageName.matches("[A-Za-z_][A-Za-z0-9_]*(?:\\.[A-Za-z_][A-Za-z0-9_]*)+")) result.put("packageName", packageName);
                    }
                    continue;
                }
                if (trimmed.startsWith("App Summary")) {
                    section = 1;
                    summary = empty(SUMMARY);
                    summary.put("rows", summaryRows);
                    result.put("appSummary", summary);
                    continue;
                }
                if (trimmed.equals("Objects") || trimmed.equals("Objects:")) {
                    section = 2;
                    objects = empty(OBJECTS);
                    objects.put("extra", new LinkedHashMap<String, Object>());
                    result.put("objects", objects);
                    continue;
                }
                if (trimmed.startsWith("Native Allocations")) {
                    section = 3;
                    allocations = empty(NATIVE);
                    allocations.put("entries", allocationRows);
                    result.put("nativeAllocations", allocations);
                    continue;
                }
                if (trimmed.equals("SQL") || trimmed.startsWith("SQL:") || trimmed.startsWith("Asset Allocations")) {
                    section = 4;
                    continue;
                }
                if (section == 1) parseSummary(line, original);
                else if (section == 2) parseObjects(line, original);
                else if (section == 3) parseAllocations(line, original);
                else if (section == 0) parseCategory(line, original);
            }
            if (result.get("pid") == null) {
                Matcher pid = PID.matcher(raw);
                if (pid.find()) result.put("pid", number(pid.group(1), null));
            }
            boolean recognized = !categories.isEmpty() || result.get("total") != null || summary != null || objects != null || allocations != null;
            if (!recognized) diagnostic("no_meminfo_sections", "warning", "No detailed meminfo sections were recognized", null);
            boolean warning = diagnostics.stream().anyMatch(item -> !item.get("severity").equals("info"));
            result.put("status", !recognized ? "unrecognized" : warning ? "partial" : "parsed");
            return result;
        }

        private void parseCategory(String line, String original) {
            String trimmed = line.trim();
            if (trimmed.matches("(?i).*\\bPss\\b.*") && (trimmed.contains("Private") || trimmed.contains("Heap") || trimmed.contains("Rss"))) {
                header = tokens(line);
                return;
            }
            if (header != null && trimmed.matches("(?i)(?:Total|Clean|Dirty|Size|Alloc|Free)(?:\\s+(?:Total|Clean|Dirty|Size|Alloc|Free))*")) {
                List<Cell> lower = tokens(line);
                if (lower.size() == header.size()) {
                    columns.clear();
                    for (int i = 0; i < lower.size(); i++) columns.add(new Column(columnKey(header.get(i).text(), lower.get(i).text()), lower.get(i).end()));
                } else diagnostic("category_header_mismatch", "warning", "Upper and lower category headers have different column counts", original);
                header = null;
                return;
            }
            String name = null;
            List<Cell> values = new ArrayList<>();
            int[] indexes = null;
            if (columns.size() > 1 && columns.get(0).end() > 0) {
                int width = columns.get(1).end() - columns.get(0).end();
                int boundary = columns.get(0).end() - width;
                if (boundary > 0 && line.length() > boundary) {
                    String prefix = line.substring(0, boundary).trim();
                    Matcher fixed = TOKEN.matcher(line);
                    fixed.region(boundary, line.length());
                    while (fixed.find()) values.add(new Cell(fixed.group(), fixed.end()));
                    indexes = positions(values, columns);
                    if (!values.isEmpty() && indexes != null && !prefix.isEmpty() && !prefix.contains(":")) name = prefix;
                    else values.clear();
                }
            }
            if (name == null) {
                Matcher row = ROW.matcher(line);
                if (!row.find()) return;
                name = row.group(1).trim();
                if (name.isEmpty() || name.contains(":")) return;
                Matcher tokens = TOKEN.matcher(line);
                tokens.region(row.start(2), line.length());
                while (tokens.find()) values.add(new Cell(tokens.group(), tokens.end()));
                indexes = null;
            }
            if (columns.isEmpty()) {
                if (values.size() != 5 && values.size() != 8) return;
                if (!assumedColumns) diagnostic("category_header_missing", "warning", "Assuming upstream PSS/private/swap/RSS column order because no table header was found", original);
                assumedColumns = true;
                for (int i = 0; i < CATEGORY.length; i++) columns.add(new Column(CATEGORY[i], -1));
            }
            Map<String, Object> item = empty(CATEGORY);
            if (!name.equals("TOTAL")) item.put("name", name);
            Map<String, Object> extra = new LinkedHashMap<>();
            item.put("extraColumns", extra);
            if (indexes == null) indexes = positions(values, columns);
            if (indexes == null) {
                indexes = new int[values.size()];
                for (int i = 0; i < indexes.length; i++) indexes[i] = i;
                if (columns.get(0).end() >= 0 && values.size() != columns.size() && values.size() != 5) {
                    diagnostic("category_cell_alignment", "warning", "Compact or malformed category row; values assigned in declared column order", original);
                }
            }
            for (int i = 0; i < values.size(); i++) {
                int index = indexes[i];
                if (index >= columns.size()) {
                    diagnostic("extra_category_value", "warning", "Unexpected category value beyond declared columns", original);
                    extra.put("column" + (index + 1), number(values.get(i).text(), original));
                    continue;
                }
                String key = columns.get(index).key();
                Long value = number(values.get(i).text(), original);
                if (item.containsKey(key)) item.put(key, value);
                else extra.put(key, value);
            }
            if (name.equals("TOTAL")) result.put("total", item);
            else categories.add(item);
        }

        private int[] positions(List<Cell> values, List<Column> layout) {
            int[] indexes = new int[values.size()];
            int previous = -1;
            for (int i = 0; i < values.size(); i++) {
                int best = -1;
                int distance = Integer.MAX_VALUE;
                for (int j = previous + 1; j < layout.size(); j++) {
                    if (layout.get(j).end() < 0) continue;
                    int candidate = Math.abs(layout.get(j).end() - values.get(i).end());
                    if (candidate < distance) {
                        best = j;
                        distance = candidate;
                    }
                }
                if (best < 0 || distance > 2) return null;
                indexes[i] = best;
                previous = best;
            }
            return indexes;
        }

        private void parseSummary(String line, String original) {
            Matcher headings = SUMMARY_HEADER.matcher(line);
            boolean hasHeader = false;
            while (headings.find()) {
                if (headings.group(1).equalsIgnoreCase("Pss")) pssEnd = headings.end();
                else rssEnd = headings.end();
                hasHeader = true;
            }
            if (hasHeader) return;
            Matcher total = TOTAL_PAIR.matcher(line);
            boolean totals = false;
            while (total.find()) {
                String label = normalized(total.group(1));
                String key = label.contains("swap") ? "totalSwapPss" : label.contains("rss") ? "totalRss" : "totalPss";
                summary.put(key, number(total.group(2), original));
                totals = true;
            }
            if (totals) return;
            int colon = line.indexOf(':');
            if (colon < 0) return;
            String label = line.substring(0, colon).trim();
            List<Cell> values = new ArrayList<>();
            Matcher matcher = TOKEN.matcher(line);
            matcher.region(colon + 1, line.length());
            while (matcher.find()) values.add(new Cell(matcher.group(), matcher.end()));
            Long pss = null;
            Long rss = null;
            if (values.size() >= 2) {
                pss = number(values.get(0).text(), original);
                rss = number(values.get(1).text(), original);
                if (values.size() > 2) diagnostic("extra_summary_value", "warning", "App Summary row has more than two values", original);
            } else if (values.size() == 1) {
                Long value = number(values.get(0).text(), original);
                String key = normalized(label);
                boolean rssOnly = key.equals("unknown") || rssEnd >= 0 && pssEnd >= 0 && Math.abs(values.get(0).end() - rssEnd) < Math.abs(values.get(0).end() - pssEnd);
                if (rssOnly) rss = value;
                else pss = value;
            } else diagnostic("missing_summary_value", "info", "App Summary row has no numeric values", original);
            Map<String, Object> item = new LinkedHashMap<>();
            item.put("name", label);
            item.put("pss", pss);
            item.put("rss", rss);
            summaryRows.add(item);
            String prefix = switch (normalized(label)) {
                case "javaheap" -> "javaHeap";
                case "nativeheap" -> "nativeHeap";
                case "code" -> "code";
                case "stack" -> "stack";
                case "graphics" -> "graphics";
                default -> null;
            };
            if (prefix != null) {
                summary.put(prefix + "Pss", pss);
                summary.put(prefix + "Rss", rss);
            } else if (normalized(label).equals("privateother")) summary.put("privateOther", pss);
            else if (normalized(label).equals("system")) summary.put("system", pss);
            else if (normalized(label).equals("unknown")) summary.put("unknownRss", rss);
        }

        @SuppressWarnings("unchecked")
        private void parseObjects(String line, String original) {
            Matcher matcher = PAIR.matcher(line);
            boolean found = false;
            while (matcher.find()) {
                found = true;
                String label = matcher.group(1).trim();
                String key = null;
                for (String candidate : OBJECTS) if (normalized(candidate).equals(normalized(label))) key = candidate;
                Long value = number(matcher.group(2), original);
                if (key == null) ((Map<String, Object>) objects.get("extra")).put(label, value);
                else objects.put(key, value);
            }
            if (!found && line.contains(":")) diagnostic("unparsed_object_row", "warning", "Objects row could not be parsed", original);
        }

        private void parseAllocations(String line, String original) {
            String trimmed = line.trim();
            if (trimmed.matches("(?i).*\\bCount\\b.*") && trimmed.matches("(?i).*\\bTotal.*")) return;
            String label;
            String rest;
            int colon = line.indexOf(':');
            if (colon >= 0) {
                label = line.substring(0, colon).trim();
                rest = line.substring(colon + 1);
            } else {
                Matcher row = ROW.matcher(line);
                if (!row.find()) return;
                label = row.group(1).trim();
                rest = line.substring(row.start(2));
            }
            List<Cell> values = tokens(rest);
            Long count = values.isEmpty() ? null : number(values.get(0).text(), original);
            Long size = values.size() < 2 ? null : number(values.get(1).text(), original);
            if (values.size() != 2) diagnostic("native_allocation_columns", "warning", "Native allocation row requires count and total KiB", original);
            Map<String, Object> item = new LinkedHashMap<>();
            item.put("name", label);
            item.put("count", count);
            item.put("totalKb", size);
            allocationRows.add(item);
            String key = normalized(label);
            if (key.equals("bitmapmalloced")) {
                allocations.put("bitmapCount", count);
                allocations.put("bitmapTotalKb", size);
            } else if (key.equals("othermalloced")) {
                allocations.put("otherMallocedCount", count);
                allocations.put("otherMallocedKb", size);
            } else if (key.equals("othernonmalloced")) {
                allocations.put("otherNonMallocedCount", count);
                allocations.put("otherNonMallocedKb", size);
            }
        }

        private Long number(String token, String line) {
            if (token == null || token.isBlank() || token.equals("-") || token.equals("--") || token.equalsIgnoreCase("N/A")) return null;
            if (!token.matches("[+-]?(?:\\d+|\\d{1,3}(?:,\\d{3})+)")) {
                diagnostic("invalid_number", "warning", "Invalid integer: " + token, line);
                return null;
            }
            try {
                long value = Long.parseLong(token.replace(",", ""));
                if (value < 0) diagnostic("negative_value", "warning", "Negative memory/count value retained", line);
                return value;
            } catch (NumberFormatException failure) {
                diagnostic("integer_overflow", "warning", "Integer exceeds signed 64-bit range: " + token, line);
                return null;
            }
        }

        private void diagnostic(String code, String severity, String message, String text) {
            Map<String, Object> item = new LinkedHashMap<>();
            item.put("code", code);
            item.put("severity", severity);
            item.put("line", lineNumber == 0 || text == null ? null : lineNumber);
            item.put("message", message);
            item.put("text", text);
            diagnostics.add(item);
        }
    }
}
