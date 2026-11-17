package dev.ayrvokan.rsm.process;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

public final class ProcessParser {
    private static final String NUMBER = "[-+]?[0-9][0-9,]*(?:\\.[0-9]+)?";
    private static final Pattern SERVICE_HEADER = Pattern.compile("^\\s*(?:\\*\\s*)?ServiceRecord\\{");
    private static final Pattern COMPONENT = Pattern.compile("([A-Za-z0-9_.]+)/([A-Za-z0-9_.$]+(?::[0-9]+)?)");
    private static final Pattern PROCESS_RECORD = Pattern.compile("ProcessRecord\\{[^\\s}]+\\s+(\\d+):([^/\\s]+)/([^\\s}]+)");
    private static final Pattern LRU_PROCESS = Pattern.compile("(\\d+):([^/\\s]+)/([^\\s}]+)");
    private static final Pattern LRU_HEADER = Pattern.compile("^\\s*(?:Proc\\s+)?#\\s*\\d+:\\s*(.*?)$");
    private static final Pattern UID = Pattern.compile("^u(\\d+)(ai|a|i|s)(\\d+)$");
    private static final Pattern USER = Pattern.compile("(?:^|\\s)u(\\d+)(?=\\s|})");
    private static final Pattern MEMORY_LINE = Pattern.compile("^\\s*(" + NUMBER + ")\\s*K(?:B|iB)?\\s*:\\s*(.+?)\\s*(?:\\(pid\\s+(\\d+)[^)]*\\))?\\s*$", Pattern.CASE_INSENSITIVE);
    private static final Pattern MEMORY_WITH_PID = Pattern.compile("^\\s*(" + NUMBER + ")\\s*K(?:B|iB)?\\s*:\\s*(.*?)\\s+\\(pid\\s+(\\d+)(?:\\s+[^)]*)?\\)\\s*$", Pattern.CASE_INSENSITIVE);
    private static final Pattern KEY_VALUE = Pattern.compile("(?:^|\\s)([A-Za-z][A-Za-z0-9_]*)=");
    private static final String[] SYSTEM_FIELDS = {"totalRamKb", "totalRamStatus", "freeRamKb", "cachedPssKb", "cachedKernelKb", "actualFreeKb", "usedRamKb", "usedPssKb", "kernelKb", "gpuKb", "lostRamKb", "zramPhysicalKb", "zramSwapKb", "zramTotalSwapKb", "oomKb", "restoreLimitKb"};
    private static final String[] SERVICE_FIELDS = {"user", "pid", "processName", "serviceName", "packageName", "isSystemApp", "serviceClass", "ramInKb", "intent", "baseDir", "dataDir", "isForeground", "foregroundId", "createTime", "lastActivityTime", "startRequested", "createdFromFg", "rawServiceRecord", "uid", "uidToken", "recentCallingUid", "appProcessRecord"};

    private ProcessParser() {}

    public static Map<String, Object> parse(String services, String lru, String meminfo) {
        Snapshot snapshot = new Snapshot();
        snapshot.parseServices(services);
        snapshot.parseLru(lru);
        snapshot.parseMemory(meminfo);
        return snapshot.finish(services, lru, meminfo);
    }

    private static Map<String, Object> map(Object... pairs) {
        Map<String, Object> result = new LinkedHashMap<>();
        for (int index = 0; index < pairs.length; index += 2) result.put((String) pairs[index], pairs[index + 1]);
        return result;
    }

    private static Map<String, Object> nullable(String... keys) {
        Map<String, Object> result = new LinkedHashMap<>();
        for (String key : keys) result.put(key, null);
        return result;
    }

    private static boolean present(String text) {
        return text != null && !text.isBlank();
    }

    private static String text(Object value) {
        return value == null ? null : value.toString();
    }

    private static String packageName(String processName) {
        if (processName == null) return null;
        int colon = processName.indexOf(':');
        return colon >= 0 ? processName.substring(0, colon) : processName;
    }

    private static Long integer(String value) {
        try {
            return value == null ? null : Long.valueOf(value);
        } catch (NumberFormatException error) {
            return null;
        }
    }

    private static Double number(String value) {
        try {
            if (value == null) return null;
            double parsed = Double.parseDouble(value.replace(",", ""));
            return Double.isFinite(parsed) ? parsed : null;
        } catch (NumberFormatException error) {
            return null;
        }
    }

    private static Long uid(String token) {
        Long direct = integer(token);
        if (direct != null && direct >= 0) return direct;
        if (token == null) return null;
        if (token.matches("s[0-9]+")) return integer(token.substring(1));
        Matcher isolated = Pattern.compile("^u(\\d+)[as]\\d+i(-?\\d+)$").matcher(token);
        if (isolated.matches()) {
            Long owner = integer(isolated.group(1));
            Long offset = integer(isolated.group(2));
            if (owner == null || offset == null) return null;
            try {
                long appId = Math.addExact(99000, offset);
                return appId >= 0 && appId < 100000 ? Math.addExact(Math.multiplyExact(owner, 100000), appId) : null;
            } catch (ArithmeticException error) {
                return null;
            }
        }
        Matcher matcher = UID.matcher(token);
        if (!matcher.matches()) return null;
        Long user = integer(matcher.group(1));
        Long suffix = integer(matcher.group(3));
        if (user == null || suffix == null) return null;
        long base = switch (matcher.group(2)) {
            case "a" -> 10000;
            case "i" -> 99000;
            case "ai" -> 90000;
            default -> 0;
        };
        try {
            return Math.addExact(Math.multiplyExact(user, 100000L), Math.addExact(base, suffix));
        } catch (ArithmeticException error) {
            return null;
        }
    }

    private static String user(Long value, String token) {
        if (token != null) {
            Matcher matcher = UID.matcher(token);
            if (matcher.matches()) return matcher.group(1);
        }
        return value == null ? null : Long.toString(value / 100000);
    }

    private static String shortService(String value) {
        return value != null && value.startsWith(".") ? value.substring(1) : value;
    }

    private static int indentation(String value) {
        int count = 0;
        while (count < value.length() && Character.isWhitespace(value.charAt(count))) count++;
        return count;
    }

    @SuppressWarnings("unchecked")
    private static List<Map<String, Object>> records(Object value) {
        return (List<Map<String, Object>>) value;
    }

    private static final class Snapshot {
        private final List<Map<String, Object>> services = new ArrayList<>();
        private final List<Map<String, Object>> lruRecords = new ArrayList<>();
        private final List<Map<String, Object>> memoryRecords = new ArrayList<>();
        private final List<Map<String, Object>> bindings = new ArrayList<>();
        private final List<Map<String, Object>> diagnostics = new ArrayList<>();
        private final Map<String, Object> system = nullable(SYSTEM_FIELDS);
        private final Map<Long, Double> pidMemory = new LinkedHashMap<>();

        private void diagnostic(String source, String code, Integer line, String message, String raw) {
            diagnostics.add(map("source", source, "code", code, "line", line, "message", message, "rawLine", raw));
        }

        private void parseServices(String input) {
            if (!present(input)) return;
            String[] lines = input.split("\\R", -1);
            int start = -1;
            List<String> block = new ArrayList<>();
            for (int index = 0; index < lines.length; index++) {
                String line = lines[index];
                boolean header = SERVICE_HEADER.matcher(line).find();
                boolean section = line.strip().matches("(?:User \\d+ active services:|Connection bindings to services:|(?:Restarting|Pending|Destroying|Delayed start) services:).*");
                if (header || section) {
                    if (start >= 0) parseService(block, start + 1);
                    block.clear();
                    start = header ? index : -1;
                }
                if (start >= 0) block.add(line);
            }
            if (start >= 0) parseService(block, start + 1);
            if (services.isEmpty() && !input.contains("ACTIVITY MANAGER SERVICES") && !input.toLowerCase(Locale.ROOT).contains("no services")) {
                diagnostic("services", "no_service_records", null, "Input contains no recognized service records", input);
            }
        }

        private void parseService(List<String> lines, int sourceLine) {
            if (lines.isEmpty()) return;
            Map<String, Object> service = nullable(SERVICE_FIELDS);
            List<String> observed = new ArrayList<>();
            List<Map<String, Object>> connections = new ArrayList<>();
            service.put("connections", connections);
            service.put("hasBound", false);
            service.put("observedFields", observed);
            service.put("sourceLine", sourceLine);
            service.put("rawServiceRecord", String.join("\n", lines) + "\n");
            Matcher component = COMPONENT.matcher(lines.get(0));
            if (component.find()) {
                service.put("packageName", component.group(1));
                service.put("serviceName", shortService(component.group(2)));
                service.put("serviceClass", component.group(2).startsWith(".") ? component.group(1) + component.group(2) : component.group(2));
                observed.add("packageName");
                observed.add("serviceName");
            }
            Matcher userMatcher = USER.matcher(lines.get(0));
            if (userMatcher.find()) {
                service.put("user", userMatcher.group(1));
                observed.add("user");
            }
            int mainIndent = Integer.MAX_VALUE;
            for (int index = 1; index < lines.size(); index++) {
                String line = lines.get(index);
                if (line.stripLeading().matches("(?:packageName|processName|baseDir|dataDir|app|intent)=.*")) mainIndent = Math.min(mainIndent, indentation(line));
            }
            for (int index = 1; index < lines.size(); index++) {
                String line = lines.get(index);
                String trimmed = line.strip();
                if (trimmed.contains("ConnectionRecord{")) {
                    String connectionText = trimmed;
                    int next = index + 1;
                    while (next < lines.size() && !lines.get(next).isBlank() && indentation(lines.get(next)) > indentation(line) && !lines.get(next).contains("ConnectionRecord{")) {
                        connectionText += "\n" + lines.get(next).strip();
                        next++;
                    }
                    Map<String, Object> connection = connection(connectionText, sourceLine + index, service);
                    if (connection != null) connections.add(connection);
                    continue;
                }
                Matcher bound = Pattern.compile("(?:^|\\s)hasBound=(true|false)(?=\\s|$|})").matcher(trimmed);
                if (bound.find()) {
                    if (bound.group(1).equals("true")) service.put("hasBound", true);
                    if (!observed.contains("hasBound")) observed.add("hasBound");
                }
                if (trimmed.startsWith("app=ProcessRecord{")) {
                    Matcher process = PROCESS_RECORD.matcher(trimmed);
                    if (process.find()) {
                        Long pid = integer(process.group(1));
                        String token = process.group(3);
                        Long value = uid(token);
                        service.put("pid", pid);
                        service.put("uid", value);
                        service.put("uidToken", token);
                        service.put("appProcessRecord", process.group(2));
                        if (service.get("processName") == null) service.put("processName", process.group(2));
                        if (service.get("user") == null) service.put("user", user(value, token));
                        for (String key : List.of("pid", "uidToken", "appProcessRecord")) if (!observed.contains(key)) observed.add(key);
                        if (value != null && !observed.contains("uid")) observed.add("uid");
                        if (value == null) diagnostic("services", "unknown_uid", sourceLine + index, "UID token could not be decoded", line);
                    } else {
                        diagnostic("services", "malformed_process_record", sourceLine + index, "ProcessRecord could not be decoded", line);
                    }
                }
                if (indentation(line) > mainIndent) continue;
                Matcher fields = KEY_VALUE.matcher(trimmed);
                List<String> keys = new ArrayList<>();
                List<Integer> starts = new ArrayList<>();
                List<Integer> ends = new ArrayList<>();
                while (fields.find()) {
                    keys.add(fields.group(1));
                    starts.add(fields.end());
                    ends.add(fields.start());
                }
                for (int field = 0; field < keys.size(); field++) {
                    String key = keys.get(field);
                    int end = field + 1 < keys.size() ? ends.get(field + 1) : trimmed.length();
                    String value = trimmed.substring(starts.get(field), end).strip();
                    String outputKey = key.equals("lastActivity") ? "lastActivityTime" : key;
                    if (List.of("packageName", "processName", "baseDir", "dataDir", "createTime", "lastActivity", "lastActivityTime", "intent").contains(key)) {
                        if (key.equals("intent")) value = trimmed.substring(starts.get(field));
                        service.put(outputKey, value);
                        if (!observed.contains(outputKey)) observed.add(outputKey);
                    } else if (List.of("isForeground", "startRequested", "createdFromFg").contains(key)) {
                        String token = value.split("\\s+", 2)[0];
                        if (token.equals("true") || token.equals("false")) {
                            service.put(outputKey, Boolean.valueOf(token));
                            if (!observed.contains(outputKey)) observed.add(outputKey);
                        } else diagnostic("services", "invalid_boolean", sourceLine + index, "Invalid " + key, line);
                    } else if (List.of("foregroundId", "recentCallingUid").contains(key)) {
                        Long parsed = integer(value.split("\\s+", 2)[0]);
                        if (parsed == null) diagnostic("services", "invalid_integer", sourceLine + index, "Invalid " + key, line);
                        else {
                            service.put(outputKey, parsed);
                            if (!observed.contains(outputKey)) observed.add(outputKey);
                        }
                    }
                }
            }
            String packageName = text(service.get("packageName"));
            if (packageName == null || service.get("serviceName") == null) {
                diagnostic("services", "malformed_service_record", sourceLine, "Missing package or service component", String.join("\n", lines));
                return;
            }
            if (service.get("processName") == null) service.put("processName", packageName);
            Long serviceUid = (Long) service.get("uid");
            String baseDir = text(service.get("baseDir"));
            boolean systemApp = !packageName.contains(".") || (serviceUid != null && serviceUid % 100000 < 10000) || (baseDir != null && (baseDir.startsWith("/system") || baseDir.startsWith("/product") || baseDir.startsWith("/system_ext")));
            service.put("isSystemApp", systemApp);
            service.put("isStoppable", Boolean.TRUE.equals(service.get("startRequested")) || !Boolean.TRUE.equals(service.get("hasBound")));
            for (Map<String, Object> connection : connections) {
                connection.put("ownerPackageName", packageName);
                connection.put("ownerServiceName", service.get("serviceName"));
                bindings.add(connection);
            }
            services.add(service);
        }

        private Map<String, Object> connection(String line, int number, Map<String, Object> service) {
            Matcher match = Pattern.compile("([A-Za-z0-9_.]+)/([A-Za-z0-9_.$]+(?::[0-9]+)?):@([A-Fa-f0-9]+)").matcher(line);
            if (!match.find()) {
                diagnostic("services", "malformed_connection_record", number, "ConnectionRecord component unavailable", line);
                return null;
            }
            Matcher flags = Pattern.compile("flags=(0x[A-Fa-f0-9]+)").matcher(line);
            String flagValue = flags.find() ? flags.group(1) : null;
            if (flagValue == null) diagnostic("services", "missing_connection_flags", number, "Connection flags unavailable", line);
            Matcher binding = Pattern.compile("binding=AppBindRecord\\{[^}]*?/[A-Za-z0-9_.$]+(?::[0-9]+)?:([^}\\s]+)").matcher(line);
            String bindingProcess = binding.find() ? binding.group(1) : null;
            return map("packageName", match.group(1), "serviceName", shortService(match.group(2)), "bindingPackage", packageName(bindingProcess), "bindingProcess", bindingProcess, "conn", match.group(3), "flags", flagValue, "isForeground", token(line, "FGS") || token(line, "FG"), "isVisible", token(line, "VIS"), "hasCapabilities", token(line, "CAPS"), "rawConnectionRecord", line, "ownerPackageName", service.get("packageName"), "ownerServiceName", service.get("serviceName"));
        }

        private boolean token(String line, String token) {
            return Pattern.compile("(?:^|\\s)" + Pattern.quote(token) + "(?=\\s|})").matcher(line).find();
        }

        private void parseLru(String input) {
            if (!present(input)) return;
            String[] lines = input.split("\\R", -1);
            for (int index = 0; index < lines.length; index++) {
                String line = lines[index];
                if (!line.stripLeading().matches("(?:Proc\\s+)?#.*")) continue;
                Matcher process = LRU_PROCESS.matcher(line);
                if (!process.find()) {
                    diagnostic("lru", "malformed_lru_record", index + 1, "PID/process/UID tuple unavailable", line);
                    continue;
                }
                Long pid = integer(process.group(1));
                if (pid == null || pid <= 0) {
                    diagnostic("lru", "invalid_pid", index + 1, "LRU PID must be positive", line);
                    continue;
                }
                Matcher header = LRU_HEADER.matcher(line.substring(0, process.start()));
                String[] tokens = header.find() && !header.group(1).isBlank() ? header.group(1).strip().split("\\s+") : new String[0];
                String state = tokens.length > 0 ? tokens[0] : null;
                int adjIndex = 1;
                if (state != null && state.matches("[A-Za-z]+[+-]") && tokens.length > 1 && tokens[1].matches("[0-9]+")) {
                    state += tokens[1];
                    adjIndex = 2;
                }
                String name = process.group(2);
                String uidToken = process.group(3);
                Long value = uid(uidToken);
                if (value == null) diagnostic("lru", "unknown_uid", index + 1, "UID token could not be decoded", line);
                String processStateLabel = tokens.length > adjIndex ? tokens[adjIndex] : null;
                lruRecords.add(map("packageName", packageName(name), "processName", name, "pid", pid, "uid", value, "uidToken", uidToken, "user", user(value, uidToken), "state", state, "adj", processStateLabel, "oomAdjustment", state, "processStateLabel", processStateLabel, "lineNumber", index + 1, "rawLine", line));
            }
            if (lruRecords.isEmpty() && !input.toLowerCase(Locale.ROOT).contains("lru") && !input.toLowerCase(Locale.ROOT).contains("no processes")) diagnostic("lru", "no_lru_records", null, "Input contains no recognized LRU records", input);
        }

        private void parseMemory(String input) {
            if (!present(input)) return;
            String[] lines = input.split("\\R", -1);
            boolean pss = false;
            boolean foundPss = false;
            Map<String, Map<String, Object>> seen = new LinkedHashMap<>();
            for (int index = 0; index < lines.length; index++) {
                String line = lines[index];
                String trimmed = line.strip();
                if (trimmed.equals("Total PSS by process:")) {
                    pss = true;
                    foundPss = true;
                    continue;
                }
                if (trimmed.matches("Total (?:PSS|RSS|SWAP PSS) by .*") || trimmed.matches("(?:Total RAM|Free RAM|Used RAM|Lost RAM|ZRAM|GPU|Tuning):.*")) pss = false;
                if (pss && !trimmed.isEmpty()) {
                    Matcher matcher = MEMORY_WITH_PID.matcher(line);
                    boolean withPid = matcher.matches();
                    if (!withPid) matcher = MEMORY_LINE.matcher(line);
                    if (withPid || matcher.matches()) {
                        Double ram = number(matcher.group(1));
                        String name = matcher.group(2).strip();
                        Long pid = integer(matcher.group(3));
                        if (ram == null || ram < 0 || name.isEmpty() || name.contains("(pid")) {
                            diagnostic("meminfo", "invalid_pss_record", index + 1, "Invalid process PSS value or PID", line);
                            continue;
                        }
                        if (pid != null && pid <= 0) {
                            diagnostic("meminfo", "invalid_pid", index + 1, "PSS PID must be positive", line);
                            continue;
                        }
                        Map<String, Object> record = map("packageName", packageName(name), "processName", name, "pid", pid, "ramKb", ram, "lineNumber", index + 1, "rawLine", line);
                        String identity = pid == null ? "name:" + name : "pid:" + pid;
                        Map<String, Object> previous = seen.putIfAbsent(identity, record);
                        if (previous != null) {
                            diagnostic("meminfo", "duplicate_pss_record", index + 1, "Repeated process identity; first PSS measurement retained", line);
                            continue;
                        }
                        memoryRecords.add(record);
                        if (pid != null) pidMemory.put(pid, ram);
                    } else {
                        diagnostic("meminfo", "unmatched_pss_line", index + 1, "Line in process PSS section could not be decoded", line);
                    }
                }
                parseSystemLine(line, index + 1);
            }
            if (!foundPss) diagnostic("meminfo", "missing_pss_section", null, "Total PSS by process section not found; system RAM fields may still be available", null);
        }

        private void parseSystemLine(String line, int number) {
            String trimmed = line.strip();
            if (trimmed.startsWith("Total RAM:")) {
                metric(line, "Total RAM:\\s*(" + NUMBER + ")K", "totalRamKb");
                Matcher status = Pattern.compile("\\(status\\s+([^)]*)\\)").matcher(line);
                if (status.find()) system.put("totalRamStatus", status.group(1).strip());
                missingMetric("totalRamKb", line, number);
            } else if (trimmed.startsWith("Free RAM:")) {
                metric(line, "Free RAM:\\s*(" + NUMBER + ")K", "freeRamKb");
                metric(line, "(" + NUMBER + ")K\\s+cached pss", "cachedPssKb");
                metric(line, "(" + NUMBER + ")K\\s+cached kernel", "cachedKernelKb");
                metric(line, "(" + NUMBER + ")K\\s+free(?:\\s|\\))", "actualFreeKb");
                missingMetric("freeRamKb", line, number);
            } else if (trimmed.startsWith("Used RAM:")) {
                metric(line, "Used RAM:\\s*(" + NUMBER + ")K", "usedRamKb");
                metric(line, "(" + NUMBER + ")K\\s+used pss", "usedPssKb");
                metric(line, "(" + NUMBER + ")K\\s+kernel(?:\\s|\\))", "kernelKb");
                metric(line, "(" + NUMBER + ")K\\s+gpu(?:\\s|\\))", "gpuKb");
                missingMetric("usedRamKb", line, number);
            } else if (trimmed.startsWith("GPU:")) {
                metric(line, "GPU:\\s*(" + NUMBER + ")K", "gpuKb");
                missingMetric("gpuKb", line, number);
            } else if (trimmed.startsWith("Lost RAM:")) {
                metric(line, "Lost RAM:\\s*(" + NUMBER + ")K", "lostRamKb");
                missingMetric("lostRamKb", line, number);
            } else if (trimmed.startsWith("ZRAM:")) {
                metric(line, "ZRAM:\\s*(" + NUMBER + ")K\\s+physical", "zramPhysicalKb");
                metric(line, "for\\s+(" + NUMBER + ")K\\s+in swap", "zramSwapKb");
                metric(line, "(" + NUMBER + ")K\\s+total swap", "zramTotalSwapKb");
                missingMetric("zramPhysicalKb", line, number);
            } else if (trimmed.startsWith("Tuning:")) {
                metric(line, "oom\\s+(" + NUMBER + ")K", "oomKb");
                metric(line, "restore limit\\s+(" + NUMBER + ")K", "restoreLimitKb");
            }
        }

        private void metric(String line, String regex, String key) {
            Matcher matcher = Pattern.compile(regex).matcher(line);
            if (matcher.find()) system.put(key, number(matcher.group(1)));
        }

        private void missingMetric(String key, String line, int number) {
            if (system.get(key) == null) diagnostic("meminfo", "invalid_system_metric", number, "Unable to parse " + key, line);
        }

        private Map<String, Object> process(Map<String, Map<String, Object>> processes, String name, Long pid) {
            if (pid == null) {
                List<Map<String, Object>> matches = processes.values().stream().filter(value -> name.equals(value.get("processName"))).toList();
                if (matches.size() == 1) return matches.get(0);
            }
            String identity = pid == null ? "name:" + name : "pid:" + pid;
            return processes.computeIfAbsent(identity, ignored -> map("packageName", packageName(name), "processName", name, "pid", pid, "uid", null, "uidToken", null, "user", null, "state", null, "adj", null, "oomAdjustment", null, "processStateLabel", null, "ramKb", null, "sourceTypes", new ArrayList<String>(), "lruRecords", new ArrayList<Map<String, Object>>()));
        }

        @SuppressWarnings("unchecked")
        private void source(Map<String, Object> process, String source) {
            List<String> values = (List<String>) process.get("sourceTypes");
            if (!values.contains(source)) values.add(source);
        }

        private Map<String, Object> app(Map<String, Map<String, Object>> apps, String packageName) {
            return apps.computeIfAbsent(packageName, ignored -> map("packageName", packageName, "services", new ArrayList<Map<String, Object>>(), "pids", new ArrayList<Long>(), "totalRamInKb", null, "isSystemApp", null, "connections", new ArrayList<Map<String, Object>>(), "processState", null, "adjLevel", null, "oomAdjustment", null, "processStateLabel", null, "hasServices", false, "ramSources", new ArrayList<Map<String, Object>>(), "cachedMemoryKb", null, "processes", new ArrayList<Map<String, Object>>(), "isCoreApp", !packageName.contains("."), "user", null, "lruProcesses", new ArrayList<Map<String, Object>>()));
        }

        private Map<String, Object> finish(String servicesInput, String lruInput, String memoryInput) {
            Map<String, Map<String, Object>> apps = new LinkedHashMap<>();
            Map<String, Map<String, Object>> processes = new LinkedHashMap<>();
            for (Map<String, Object> service : services) {
                String packageName = text(service.get("packageName"));
                Map<String, Object> app = app(apps, packageName);
                records(app.get("services")).add(service);
                records(app.get("connections")).addAll(records(service.get("connections")));
                app.put("hasServices", true);
                if (app.get("isSystemApp") == null) app.put("isSystemApp", service.get("isSystemApp"));
                if (app.get("user") == null) app.put("user", service.get("user"));
                Long pid = (Long) service.get("pid");
                String name = text(service.get("processName"));
                if (pid != null) {
                    Map<String, Object> process = process(processes, name, pid);
                    for (String key : List.of("uid", "uidToken", "user")) if (service.get(key) != null) process.put(key, service.get(key));
                    process.put("packageName", packageName);
                    source(process, "service");
                }
                if (pid != null && pidMemory.containsKey(pid)) service.put("ramInKb", pidMemory.get(pid));
            }
            for (Map<String, Object> row : lruRecords) {
                String packageName = text(row.get("packageName"));
                Map<String, Object> app = app(apps, packageName);
                records(app.get("lruProcesses")).add(row);
                app.put("processState", row.get("state"));
                app.put("adjLevel", row.get("adj"));
                app.put("oomAdjustment", row.get("oomAdjustment"));
                app.put("processStateLabel", row.get("processStateLabel"));
                if (app.get("user") == null) app.put("user", row.get("user"));
                Map<String, Object> process = process(processes, text(row.get("processName")), (Long) row.get("pid"));
                for (String key : List.of("uid", "uidToken", "user", "state", "adj", "oomAdjustment", "processStateLabel")) if (row.get(key) != null) process.put(key, row.get(key));
                records(process.get("lruRecords")).add(row);
                source(process, "lru");
            }
            for (Map<String, Object> row : memoryRecords) {
                Map<String, Object> process = process(processes, text(row.get("processName")), (Long) row.get("pid"));
                if (process.get("ramKb") == null) process.put("ramKb", row.get("ramKb"));
                source(process, "meminfoPss");
                String packageName = text(process.get("packageName"));
                boolean memoryOnly = !apps.containsKey(packageName);
                Map<String, Object> app = app(apps, packageName);
                if (memoryOnly) app.put("isCoreApp", true);
            }
            for (Map<String, Object> process : processes.values()) {
                Map<String, Object> app = app(apps, text(process.get("packageName")));
                records(app.get("processes")).add(process);
                if (app.get("user") == null) app.put("user", process.get("user"));
            }
            for (Map<String, Object> app : apps.values()) finalizeApp(app);
            return map("schemaVersion", "rsm-process-parser/1", "memoryUnit", "KiB", "sourcePresence", map("services", present(servicesInput), "lru", present(lruInput), "meminfo", present(memoryInput)), "apps", new ArrayList<>(apps.values()), "services", services, "processes", new ArrayList<>(processes.values()), "bindings", bindings, "systemRamInfo", system, "diagnostics", diagnostics);
        }

        private void finalizeApp(Map<String, Object> app) {
            Set<Long> pids = new LinkedHashSet<>();
            Double total = null;
            List<Map<String, Object>> sources = records(app.get("ramSources"));
            Set<String> counted = new LinkedHashSet<>();
            for (Map<String, Object> process : records(app.get("processes"))) {
                Long pid = (Long) process.get("pid");
                if (pid != null) pids.add(pid);
                Double ram = (Double) process.get("ramKb");
                String identity = pid == null ? "name:" + process.get("processName") : "pid:" + pid;
                if (ram != null && counted.add(identity)) {
                    total = (total == null ? 0 : total) + ram;
                    sources.add(map("source", "meminfoPss", "ramKb", ram, "pid", pid, "processName", process.get("processName")));
                }
            }
            for (Map<String, Object> service : records(app.get("services"))) {
                Long pid = (Long) service.get("pid");
                if (pid != null) pids.add(pid);
            }
            app.put("pids", new ArrayList<>(pids));
            app.put("totalRamInKb", total);
            boolean hasActiveService = records(app.get("services")).stream().anyMatch(service -> Boolean.TRUE.equals(service.get("startRequested")) || Boolean.TRUE.equals(service.get("isForeground")) || Boolean.TRUE.equals(service.get("hasBound")) || service.get("pid") != null);
            app.put("hasActiveService", hasActiveService);
            String state = text(app.get("processState"));
            boolean active = hasActiveService;
            if (!active && state == null) active = Boolean.TRUE.equals(app.get("hasServices"));
            if (!active && state != null) {
                String value = state.toLowerCase(Locale.ROOT);
                for (String prefix : List.of("fg", "vis", "prev", "prcp", "svcb", "home", "hvy", "psvc", "pers", "top", "btop", "impf", "bfgs", "rcvr")) if (value.startsWith(prefix)) active = true;
            }
            app.put("isActive", active);
            app.put("isCached", state != null && state.toLowerCase(Locale.ROOT).startsWith("cch"));
            app.put("processCount", pids.isEmpty() ? records(app.get("processes")).size() : pids.size());
        }
    }
}
