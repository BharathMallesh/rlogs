package org.apache.logging.log4j.core;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import org.yaml.snakeyaml.Yaml;

/**
 * Converts YAML and JSON configuration formats to the XML format
 * understood by the Rust native engine.
 */
public class ConfigConverter {

    /**
     * Converts a YAML configuration string to Log4j2-style XML.
     * Expected YAML structure mirrors Log4j2 XML hierarchy:
     *
     * Configuration:
     *   status: WARN
     *   Appenders:
     *     File:
     *       name: File
     *       fileName: logs/app.log
     *       JsonLayout:
     *         properties: "true"
     *   Loggers:
     *     Root:
     *       level: info
     *       AppenderRef:
     *         ref: File
     */
    @SuppressWarnings("unchecked")
    public static String yamlToXml(String yaml) {
        Yaml parser = new Yaml();
        Map<String, Object> root = parser.load(yaml);
        if (root == null) return "";

        StringBuilder sb = new StringBuilder();
        sb.append("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");

        // The top-level key should be "Configuration"
        if (root.containsKey("Configuration")) {
            mapToXml(sb, "Configuration", root.get("Configuration"), 0);
        } else {
            // Treat entire YAML as Configuration content
            mapToXml(sb, "Configuration", root, 0);
        }
        return sb.toString();
    }

    /**
     * Converts a JSON configuration string to Log4j2-style XML.
     * Uses SnakeYAML since JSON is a valid YAML subset.
     */
    public static String jsonToXml(String json) {
        // JSON is valid YAML, so we reuse the same converter
        return yamlToXml(json);
    }

    @SuppressWarnings("unchecked")
    private static void mapToXml(StringBuilder sb, String tagName, Object value, int indent) {
        String pad = repeat("  ", indent);

        if (value instanceof Map) {
            Map<String, Object> map = (Map<String, Object>) value;

            // Separate attributes (primitive values) from child elements (maps/lists)
            Map<String, String> attrs = new LinkedHashMap<>();
            Map<String, Object> children = new LinkedHashMap<>();

            for (Map.Entry<String, Object> entry : map.entrySet()) {
                Object v = entry.getValue();
                if (v instanceof String || v instanceof Number || v instanceof Boolean) {
                    attrs.put(entry.getKey(), String.valueOf(v));
                } else {
                    children.put(entry.getKey(), v);
                }
            }

            sb.append(pad).append("<").append(tagName);
            for (Map.Entry<String, String> attr : attrs.entrySet()) {
                sb.append(" ").append(attr.getKey()).append("=\"").append(escapeXml(attr.getValue())).append("\"");
            }

            if (children.isEmpty()) {
                sb.append(" />\n");
            } else {
                sb.append(">\n");
                for (Map.Entry<String, Object> child : children.entrySet()) {
                    mapToXml(sb, child.getKey(), child.getValue(), indent + 1);
                }
                sb.append(pad).append("</").append(tagName).append(">\n");
            }
        } else if (value instanceof List) {
            List<Object> list = (List<Object>) value;
            for (Object item : list) {
                mapToXml(sb, tagName, item, indent);
            }
        } else if (value != null) {
            // Primitive value as self-closing tag with value attribute
            sb.append(pad).append("<").append(tagName)
              .append(" value=\"").append(escapeXml(String.valueOf(value))).append("\" />\n");
        }
    }

    private static String escapeXml(String s) {
        return s.replace("&", "&amp;")
                .replace("<", "&lt;")
                .replace(">", "&gt;")
                .replace("\"", "&quot;");
    }

    private static String repeat(String s, int count) {
        StringBuilder sb = new StringBuilder();
        for (int i = 0; i < count; i++) sb.append(s);
        return sb.toString();
    }
}
