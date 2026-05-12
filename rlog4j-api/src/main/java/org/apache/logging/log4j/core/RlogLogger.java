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
        return NativeLogger.getEffectiveLevel(getName());
    }

    /**
     * Uses the per-logger hierarchy so that e.g. logger.debug() on a logger
     * configured at DEBUG is enabled even when Root is INFO.
     * This also enables lambda lazy logging: logger.debug(() -> expensive())
     * skips evaluation entirely when the effective level is INFO or above.
     */
    private boolean isLevelEnabled(Level level) {
        return level != null
            && level.intLevel() <= NativeLogger.getEffectiveLevel(getName()).intLevel();
    }

    @Override
    public boolean isEnabled(Level level, Marker marker, Message message, Throwable t) { return isLevelEnabled(level); }
    @Override
    public boolean isEnabled(Level level, Marker marker, CharSequence message, Throwable t) { return isLevelEnabled(level); }
    @Override
    public boolean isEnabled(Level level, Marker marker, Object message, Throwable t) { return isLevelEnabled(level); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Throwable t) { return isLevelEnabled(level); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message) { return isLevelEnabled(level); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object... params) { return isLevelEnabled(level); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object p0) { return isLevelEnabled(level); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object p0, Object p1) { return isLevelEnabled(level); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object p0, Object p1, Object p2) { return isLevelEnabled(level); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object p0, Object p1, Object p2, Object p3) { return isLevelEnabled(level); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object p0, Object p1, Object p2, Object p3, Object p4) { return isLevelEnabled(level); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object p0, Object p1, Object p2, Object p3, Object p4, Object p5) { return isLevelEnabled(level); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object p0, Object p1, Object p2, Object p3, Object p4, Object p5, Object p6) { return isLevelEnabled(level); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object p0, Object p1, Object p2, Object p3, Object p4, Object p5, Object p6, Object p7) { return isLevelEnabled(level); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object p0, Object p1, Object p2, Object p3, Object p4, Object p5, Object p6, Object p7, Object p8) { return isLevelEnabled(level); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object p0, Object p1, Object p2, Object p3, Object p4, Object p5, Object p6, Object p7, Object p8, Object p9) { return isLevelEnabled(level); }

    @Override
    public void logMessage(String fqcn, Level level, Marker marker, Message message, Throwable t) {
        int rustLevel = 3;
        if (level == Level.TRACE) rustLevel = 1;
        else if (level == Level.DEBUG) rustLevel = 2;
        else if (level == Level.INFO) rustLevel = 3;
        else if (level == Level.WARN) rustLevel = 4;
        else if (level == Level.ERROR || level == Level.FATAL) rustLevel = 5;

        // Clean message — no exception appended; %ex in the pattern handles that
        String formattedMsg = message.getFormattedMessage();

        // Full exception stack trace via %ex
        String exceptionStr = null;
        if (t != null) {
            java.io.StringWriter sw = new java.io.StringWriter(512);
            t.printStackTrace(new java.io.PrintWriter(sw));
            exceptionStr = sw.toString();
        }

        // MDC flat map
        java.util.Map<String, String> contextMap = org.apache.logging.log4j.ThreadContext.getContext();
        StringBuilder ctxBuilder = new StringBuilder();
        if (marker != null) {
            ctxBuilder.append("marker=").append(marker.getName());
        }
        if (contextMap != null && !contextMap.isEmpty()) {
            if (ctxBuilder.length() > 0) ctxBuilder.append(", ");
            String mapStr = contextMap.toString();
            ctxBuilder.append(mapStr, 1, mapStr.length() - 1); // strip { }
        }
        String mdcString = ctxBuilder.length() > 0 ? "{" + ctxBuilder + "}" : null;

        // NDC stack (space-separated; rendered by %x in pattern)
        java.util.List<String> ndcStack =
            org.apache.logging.log4j.ThreadContext.getImmutableStack().asList();
        String ndcString = ndcStack.isEmpty() ? null : String.join(" ", ndcStack);

        NativeLogger.log(rustLevel, formattedMsg, mdcString, ndcString,
                getName(), Thread.currentThread().getName(), exceptionStr);
    }
}
