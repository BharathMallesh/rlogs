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

        String formattedMsg = message.getFormattedMessage();

        // Full exception stack trace via %ex
        String exceptionStr = null;
        if (t != null) {
            java.io.StringWriter sw = new java.io.StringWriter(512);
            t.printStackTrace(new java.io.PrintWriter(sw));
            exceptionStr = sw.toString();
        }

        // MDC flat map (marker kept separate, no longer packed into MDC)
        java.util.Map<String, String> contextMap = org.apache.logging.log4j.ThreadContext.getContext();
        String mdcString = null;
        if (contextMap != null && !contextMap.isEmpty()) {
            String mapStr = contextMap.toString();
            mdcString = "{" + mapStr.substring(1, mapStr.length() - 1) + "}";
        }

        // NDC stack (space-separated; rendered by %x)
        java.util.List<String> ndcStack =
            org.apache.logging.log4j.ThreadContext.getImmutableStack().asList();
        String ndcString = ndcStack.isEmpty() ? null : String.join(" ", ndcStack);

        // Feature 9: Marker name
        String markerName = (marker != null) ? marker.getName() : null;

        // Feature 8: Caller location.
        // Match only the specific bridge classes (not the entire package) so that
        // application code that happens to live in org.apache.logging.* is not skipped.
        String callerClass = null, callerMethod = null, callerFile = null, callerLine = null;
        try {
            StackTraceElement[] stack = Thread.currentThread().getStackTrace();
            int lastFwFrame = -1;
            for (int j = 0; j < stack.length; j++) {
                String cls = stack[j].getClassName();
                if (cls.equals("java.lang.Thread")
                        || cls.equals("org.apache.logging.log4j.spi.AbstractLogger")
                        || cls.equals(RlogLogger.class.getName())) {
                    lastFwFrame = j;
                }
            }
            if (lastFwFrame >= 0 && lastFwFrame + 1 < stack.length) {
                StackTraceElement f = stack[lastFwFrame + 1];
                callerClass  = f.getClassName();
                callerMethod = f.getMethodName();
                callerFile   = f.getFileName() != null ? f.getFileName() : "?";
                callerLine   = String.valueOf(f.getLineNumber());
            }
        } catch (Exception ignored) {}

        NativeLogger.log(rustLevel, formattedMsg, mdcString, ndcString,
                getName(), Thread.currentThread().getName(), exceptionStr,
                markerName, callerClass, callerMethod, callerFile, callerLine);
    }
}
