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

public class NativeLogger {
    private static final Linker linker = Linker.nativeLinker();
    private static final SymbolLookup lookup;
    private static final MethodHandle rlogInitHandle;
    private static final MethodHandle rlogLogHandle;
    private static final MethodHandle rlogConfigureHandle;

    private static final MethodHandle rlogLogWithContextHandle;

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

            // Attempt to read log4j2.xml from classpath
            InputStream is = NativeLogger.class.getClassLoader().getResourceAsStream("log4j2.xml");
            if (is != null) {
                try (Scanner scanner = new Scanner(is, "UTF-8").useDelimiter("\\A")) {
                    String xmlContent = scanner.hasNext() ? scanner.next() : "";
                    try (Arena arena = Arena.ofConfined()) {
                        MemorySegment xmlSegment = arena.allocateFrom(xmlContent);
                        int res = (int) rlogConfigureHandle.invokeExact(xmlSegment);
                    }
                }
            }

            // Initialize the Rust logger
            int status = (int) rlogInitHandle.invokeExact();
            if (status != 0) {
                throw new RuntimeException("Failed to initialize Rust logger, status: " + status);
            }
        } catch (Throwable t) {
            throw new ExceptionInInitializerError(t);
        }
    }

    public static void log(int level, String message) {
        log(level, message, null);
    }

    public static void log(int level, String message, String mdcString) {
        // Allocate a short-lived arena for the C-String conversion
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment msgSegment = arena.allocateFrom(message);
            if (mdcString == null || mdcString.isEmpty()) {
                rlogLogHandle.invokeExact(level, msgSegment);
            } else {
                MemorySegment mdcSegment = arena.allocateFrom(mdcString);
                rlogLogWithContextHandle.invokeExact(level, msgSegment, mdcSegment);
            }
        } catch (Throwable t) {
            t.printStackTrace();
        }
    }
}
