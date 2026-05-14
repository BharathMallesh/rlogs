package org.apache.logging.log4j.core;

import org.apache.logging.log4j.Level;
import org.apache.logging.log4j.Marker;
import org.apache.logging.log4j.message.Message;
import org.apache.logging.log4j.message.MessageFactory;
import org.apache.logging.log4j.spi.AbstractLogger;

public class RlogLogger extends AbstractLogger {

    public RlogLogger(String name, MessageFactory messageFactory) {
        super(name, messageFactory);
    }

    @Override
    public Level getLevel() {
        return NativeLogger.configuredLevel;
    }

    private boolean levelEnabled(Level level) {
        return level.isMoreSpecificThan(NativeLogger.configuredLevel);
    }

    @Override
    public boolean isEnabled(Level level, Marker marker, Message message, Throwable t) { return levelEnabled(level); }
    @Override
    public boolean isEnabled(Level level, Marker marker, CharSequence message, Throwable t) { return levelEnabled(level); }
    @Override
    public boolean isEnabled(Level level, Marker marker, Object message, Throwable t) { return levelEnabled(level); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Throwable t) { return levelEnabled(level); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message) { return levelEnabled(level); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object... params) { return levelEnabled(level); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object p0) { return levelEnabled(level); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object p0, Object p1) { return levelEnabled(level); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object p0, Object p1, Object p2) { return levelEnabled(level); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object p0, Object p1, Object p2, Object p3) { return levelEnabled(level); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object p0, Object p1, Object p2, Object p3, Object p4) { return levelEnabled(level); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object p0, Object p1, Object p2, Object p3, Object p4, Object p5) { return levelEnabled(level); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object p0, Object p1, Object p2, Object p3, Object p4, Object p5, Object p6) { return levelEnabled(level); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object p0, Object p1, Object p2, Object p3, Object p4, Object p5, Object p6, Object p7) { return levelEnabled(level); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object p0, Object p1, Object p2, Object p3, Object p4, Object p5, Object p6, Object p7, Object p8) { return levelEnabled(level); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object p0, Object p1, Object p2, Object p3, Object p4, Object p5, Object p6, Object p7, Object p8, Object p9) { return levelEnabled(level); }

    @Override
    public void logMessage(String fqcn, Level level, Marker marker, Message message, Throwable t) {
        int rustLevel = 3; // Default INFO
        if (level == Level.TRACE) rustLevel = 1;
        else if (level == Level.DEBUG) rustLevel = 2;
        else if (level == Level.INFO) rustLevel = 3;
        else if (level == Level.WARN) rustLevel = 4;
        else if (level == Level.ERROR || level == Level.FATAL) rustLevel = 5;

        String formattedMsg = message.getFormattedMessage();
        if (t != null) {
            formattedMsg += " | Exception: " + t.toString();
        }
        
        // Serialize MDC as a JSON object so all output formats (JSON, XML, Logfmt, Text)
        // can parse it unambiguously in Rust. Null when the context map is empty.
        java.util.Map<String, String> contextMap = org.apache.logging.log4j.ThreadContext.getContext();
        String mdcString = (contextMap != null && !contextMap.isEmpty())
                ? mdcToJson(contextMap) : null;

        NativeLogger.log(rustLevel, formattedMsg, mdcString);
    }

    /** Serialize an MDC map to a compact JSON object string: {"key":"value",...} */
    private static String mdcToJson(java.util.Map<String, String> mdc) {
        StringBuilder sb = new StringBuilder("{");
        boolean first = true;
        for (java.util.Map.Entry<String, String> e : mdc.entrySet()) {
            if (!first) sb.append(',');
            sb.append('"').append(jsonEscape(e.getKey()))
              .append("\":\"").append(jsonEscape(e.getValue())).append('"');
            first = false;
        }
        sb.append('}');
        return sb.toString();
    }

    private static String jsonEscape(String s) {
        StringBuilder out = new StringBuilder(s.length() + 4);
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            switch (c) {
                case '"':  out.append("\\\""); break;
                case '\\': out.append("\\\\"); break;
                case '\n': out.append("\\n");  break;
                case '\r': out.append("\\r");  break;
                case '\t': out.append("\\t");  break;
                default:
                    if (c < 0x20) out.append(String.format("\\u%04x", (int) c));
                    else          out.append(c);
            }
        }
        return out.toString();
    }
}
