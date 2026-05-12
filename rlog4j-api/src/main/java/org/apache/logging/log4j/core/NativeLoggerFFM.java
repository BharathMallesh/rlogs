package org.apache.logging.log4j.core;

import java.lang.foreign.Arena;
import java.lang.foreign.FunctionDescriptor;
import java.lang.foreign.Linker;
import java.lang.foreign.MemorySegment;
import java.lang.foreign.SymbolLookup;
import java.lang.foreign.ValueLayout;
import java.lang.invoke.MethodHandle;

/**
 * FFM-based (Java 22+) bridge to the same Rust C ABI that NativeLogger calls
 * via JNI. The Rust side is unchanged — both classes invoke the same exported
 * symbols (rlog_log, rlog_log_full, rlog_flush).
 *
 * <p>Compared with the JNI path this skips:
 * <ul>
 *   <li>The JNI prologue/epilogue (GC-root setup, etc.) on every call.</li>
 *   <li>The {@code GetStringUTFChars} → temporary native buffer copy.</li>
 * </ul>
 *
 * <p>It pays one extra cost: explicit native memory allocation per call via
 * {@link Arena#ofConfined()}. For workloads where this matters, a thread-local
 * arena could be reused.
 */
public final class NativeLoggerFFM {

    private static final Linker LINKER = Linker.nativeLinker();
    private static final SymbolLookup LIB;

    private static final MethodHandle H_RLOG_LOG;
    private static final MethodHandle H_RLOG_LOG_WITH_CONTEXT;
    private static final MethodHandle H_RLOG_LOG_FULL;
    private static final MethodHandle H_RLOG_FLUSH;

    static {
        NativeLoader.load();
        java.nio.file.Path path = NativeLoader.getLibraryPath();
        if (path == null) {
            throw new ExceptionInInitializerError(
                "FFM lookup needs the native library on disk; "
              + "NativeLoader.getLibraryPath() returned null");
        }
        LIB = SymbolLookup.libraryLookup(path, Arena.global());

        H_RLOG_LOG = LINKER.downcallHandle(
            LIB.find("rlog_log").orElseThrow(),
            FunctionDescriptor.ofVoid(ValueLayout.JAVA_INT, ValueLayout.ADDRESS));

        H_RLOG_LOG_WITH_CONTEXT = LINKER.downcallHandle(
            LIB.find("rlog_log_with_context").orElseThrow(),
            FunctionDescriptor.ofVoid(ValueLayout.JAVA_INT,
                                      ValueLayout.ADDRESS, ValueLayout.ADDRESS));

        H_RLOG_LOG_FULL = LINKER.downcallHandle(
            LIB.find("rlog_log_full").orElseThrow(),
            FunctionDescriptor.ofVoid(
                ValueLayout.JAVA_INT,     // level
                ValueLayout.ADDRESS,      // message
                ValueLayout.ADDRESS,      // mdc
                ValueLayout.ADDRESS,      // ndc
                ValueLayout.ADDRESS,      // logger_name
                ValueLayout.ADDRESS,      // thread_name
                ValueLayout.ADDRESS));    // exception

        H_RLOG_FLUSH = LINKER.downcallHandle(
            LIB.find("rlog_flush").orElseThrow(),
            FunctionDescriptor.ofVoid());

        System.out.println("Rlog4 FFM: bridge initialised (" + path + ")");
    }

    private NativeLoggerFFM() {}

    // ── 2-arg log ────────────────────────────────────────────────────────────

    public static void log(int level, String message) {
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment msg = arena.allocateFrom(message);
            H_RLOG_LOG.invokeExact(level, msg);
        } catch (Throwable t) {
            System.err.println("Rlog4 FFM Log Failure: " + message);
        }
    }

    // ── 3-arg log (level, message, mdc) ─────────────────────────────────────

    public static void log(int level, String message, String mdc) {
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment msgSeg = arena.allocateFrom(message);
            MemorySegment ctxSeg = (mdc == null) ? MemorySegment.NULL
                                                 : arena.allocateFrom(mdc);
            H_RLOG_LOG_WITH_CONTEXT.invokeExact(level, msgSeg, ctxSeg);
        } catch (Throwable t) {
            System.err.println("Rlog4 FFM Log Failure: " + message);
        }
    }

    // ── Full 7-arg log (matches RlogLogger.logMessage payload) ──────────────

    public static void logFull(int level, String message,
                               String mdc, String ndc,
                               String loggerName, String threadName,
                               String exception) {
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment msgSeg = arena.allocateFrom(message);
            MemorySegment mdcSeg = arena.allocateFrom(mdc != null ? mdc : "");
            MemorySegment ndcSeg = arena.allocateFrom(ndc != null ? ndc : "");
            MemorySegment lnSeg  = arena.allocateFrom(loggerName != null ? loggerName : "");
            MemorySegment tnSeg  = arena.allocateFrom(threadName != null ? threadName
                                       : Thread.currentThread().getName());
            MemorySegment exSeg  = arena.allocateFrom(exception != null ? exception : "");
            H_RLOG_LOG_FULL.invokeExact(level, msgSeg, mdcSeg, ndcSeg, lnSeg, tnSeg, exSeg);
        } catch (Throwable t) {
            System.err.println("Rlog4 FFM Log Failure: " + message);
        }
    }

    public static void flush() {
        try { H_RLOG_FLUSH.invokeExact(); }
        catch (Throwable t) {
            System.err.println("Rlog4 FFM Flush Failure: " + t.getMessage());
        }
    }
}
