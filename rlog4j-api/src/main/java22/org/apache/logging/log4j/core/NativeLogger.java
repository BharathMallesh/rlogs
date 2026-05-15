package org.apache.logging.log4j.core;

import java.lang.foreign.Arena;
import java.lang.foreign.FunctionDescriptor;
import java.lang.foreign.Linker;
import java.lang.foreign.MemorySegment;
import java.lang.foreign.SymbolLookup;
import java.lang.foreign.ValueLayout;
import java.lang.invoke.MethodHandle;
import java.io.InputStream;
import java.util.Scanner;
import org.apache.logging.log4j.Level;

public class NativeLogger {
    private static final Linker linker = Linker.nativeLinker();
    private static final SymbolLookup lookup;
    private static final MethodHandle rlogInitHandle;
    private static final MethodHandle rlogLogHandle;
    private static final MethodHandle rlogConfigureHandle;
    private static final MethodHandle rlogLogWithContextHandle;
    private static final MethodHandle rlogFlushHandle;

    // Thread-local pre-allocated buffer (8KB) to avoid Arena malloc/free per log call
    private static final ThreadLocal<MemorySegment> THREAD_BUF =
        ThreadLocal.withInitial(() -> Arena.global().allocate(8192));
    private static final int MAX_INLINE_BYTES = 8000;

    static volatile Level configuredLevel = Level.INFO;

    static {
        NativeLoader.load();
        lookup = SymbolLookup.loaderLookup();
        try {
            rlogConfigureHandle = linker.downcallHandle(
                lookup.find("rlog_configure").orElseThrow(),
                FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS)
            );
            rlogInitHandle = linker.downcallHandle(
                lookup.find("rlog_init").orElseThrow(),
                FunctionDescriptor.of(ValueLayout.JAVA_INT)
            );
            rlogLogHandle = linker.downcallHandle(
                lookup.find("rlog_log").orElseThrow(),
                FunctionDescriptor.ofVoid(ValueLayout.JAVA_INT, ValueLayout.ADDRESS)
            );
            rlogLogWithContextHandle = linker.downcallHandle(
                lookup.find("rlog_log_with_context").orElseThrow(),
                FunctionDescriptor.ofVoid(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
            );
            rlogFlushHandle = linker.downcallHandle(
                lookup.find("rlog_flush").orElseThrow(),
                FunctionDescriptor.ofVoid()
            );

            InputStream is = NativeLogger.class.getClassLoader().getResourceAsStream("log4j2.xml");
            if (is != null) {
                try (Scanner scanner = new Scanner(is, "UTF-8").useDelimiter("\\A")) {
                    String xmlContent = scanner.hasNext() ? scanner.next() : "";
                    configuredLevel = parseRootLevel(xmlContent);
                    try (Arena arena = Arena.ofConfined()) {
                        MemorySegment xmlSegment = arena.allocateFrom(xmlContent);
                        int ignored = (int) rlogConfigureHandle.invokeExact(xmlSegment);
                    }
                }
            }

            int status = (int) rlogInitHandle.invokeExact();
            if (status != 0) {
                throw new RuntimeException("Failed to initialize Rust logger, status: " + status);
            }

            Runtime.getRuntime().addShutdownHook(new Thread(() -> {
                try { rlogFlushHandle.invokeExact(); } catch (Throwable ignored) {}
            }, "rlog-shutdown"));
        } catch (Throwable t) {
            throw new ExceptionInInitializerError(t);
        }
    }

    private static Level parseRootLevel(String xml) {
        int rootIdx = xml.indexOf("<Root");
        if (rootIdx < 0) return Level.INFO;
        int gt = xml.indexOf('>', rootIdx);
        int levelIdx = xml.indexOf("level=", rootIdx);
        if (levelIdx < 0 || levelIdx > gt) return Level.INFO;
        char quote = xml.charAt(levelIdx + 6);
        int start = levelIdx + 7;
        int end = xml.indexOf(quote, start);
        if (end < 0) return Level.INFO;
        return Level.toLevel(xml.substring(start, end), Level.INFO);
    }

    public static void log(int level, String message) {
        log(level, message, null);
    }

    public static void log(int level, String message, String mdcString) {
        try {
            byte[] msgBytes = message.getBytes(java.nio.charset.StandardCharsets.UTF_8);
            if (msgBytes.length < MAX_INLINE_BYTES && mdcString == null) {
                // Fast path: reuse thread-local buffer, zero extra allocation
                MemorySegment buf = THREAD_BUF.get();
                buf.copyFrom(MemorySegment.ofArray(msgBytes));
                buf.set(ValueLayout.JAVA_BYTE, msgBytes.length, (byte) 0);
                rlogLogHandle.invokeExact(level, buf);
            } else {
                // Slow path: message is large or has MDC — use a confined arena
                try (Arena arena = Arena.ofConfined()) {
                    MemorySegment msgSegment = arena.allocateFrom(message);
                    if (mdcString == null || mdcString.isEmpty()) {
                        rlogLogHandle.invokeExact(level, msgSegment);
                    } else {
                        MemorySegment mdcSegment = arena.allocateFrom(mdcString);
                        rlogLogWithContextHandle.invokeExact(level, msgSegment, mdcSegment);
                    }
                }
            }
        } catch (Throwable t) {
            t.printStackTrace();
        }
    }
}
