package org.apache.logging.log4j.core;

/**
 * JNI bridge to the Rust rlog_core library.  Used by Java 8–21; Java 22+
 * overrides NativeLogger via the Multi-Release JAR mechanism and goes through
 * the faster Foreign Function & Memory (FFM) API instead.
 */
class NativeLoggerJNI {

    static {
        NativeLoader.load();
    }

    static native int  rlogConfigure(String xml);
    static native int  rlogInit();
    static native void rlogLog(int level, String message, String mdc);
    static native void rlogFlush();
}
