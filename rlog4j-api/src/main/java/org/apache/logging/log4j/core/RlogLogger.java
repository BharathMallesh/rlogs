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
        return Level.ALL;
    }

    private boolean isEnabled() {
        return true;
    }

    @Override
    public boolean isEnabled(Level level, Marker marker, Message message, Throwable t) { return isEnabled(); }
    @Override
    public boolean isEnabled(Level level, Marker marker, CharSequence message, Throwable t) { return isEnabled(); }
    @Override
    public boolean isEnabled(Level level, Marker marker, Object message, Throwable t) { return isEnabled(); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Throwable t) { return isEnabled(); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message) { return isEnabled(); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object... params) { return isEnabled(); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object p0) { return isEnabled(); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object p0, Object p1) { return isEnabled(); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object p0, Object p1, Object p2) { return isEnabled(); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object p0, Object p1, Object p2, Object p3) { return isEnabled(); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object p0, Object p1, Object p2, Object p3, Object p4) { return isEnabled(); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object p0, Object p1, Object p2, Object p3, Object p4, Object p5) { return isEnabled(); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object p0, Object p1, Object p2, Object p3, Object p4, Object p5, Object p6) { return isEnabled(); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object p0, Object p1, Object p2, Object p3, Object p4, Object p5, Object p6, Object p7) { return isEnabled(); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object p0, Object p1, Object p2, Object p3, Object p4, Object p5, Object p6, Object p7, Object p8) { return isEnabled(); }
    @Override
    public boolean isEnabled(Level level, Marker marker, String message, Object p0, Object p1, Object p2, Object p3, Object p4, Object p5, Object p6, Object p7, Object p8, Object p9) { return isEnabled(); }

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
        
        // Extract MDC context
        java.util.Map<String, String> contextMap = org.apache.logging.log4j.ThreadContext.getContext();
        String mdcString = null;
        if (contextMap != null && !contextMap.isEmpty()) {
            mdcString = contextMap.toString();
        }
        
        NativeLogger.log(rustLevel, formattedMsg, mdcString);
    }
}
