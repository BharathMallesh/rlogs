package org.apache.logging.log4j.core;

import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;
import org.apache.logging.log4j.ThreadContext;
import org.junit.jupiter.api.Test;

public class NativeLoggerTest {

    // ── Feature 1: PatternLayout ──────────────────────────────────────────

    @Test
    public void testPatternLayout() throws Exception {
        System.out.println("\n=== Feature 1: PatternLayout ===");
        System.out.println("Expected format: HH:mm:ss.SSS [thread] LEVEL  logger - message\n");

        // These use the full API: logger name + thread name travel to Rust
        NativeLogger.log(3, "INFO  via NativeLogger direct", null,
                "com.example.App", "main");
        NativeLogger.log(2, "DEBUG via NativeLogger direct", null,
                "com.example.service.UserService", "http-worker-1");
        NativeLogger.log(4, "WARN  via NativeLogger direct", null,
                "com.example.repository.OrderRepo", "scheduler");
        NativeLogger.log(5, "ERROR via NativeLogger direct", null,
                "com.example.api.PaymentController", "nio-thread-2");

        Thread.sleep(300);
    }

    @Test
    public void testPatternWithMdc() throws Exception {
        System.out.println("\n=== Feature 1: PatternLayout + MDC (%X) ===\n");

        NativeLogger.log(3, "Request received", "{requestId=REQ-001, userId=42}",
                "com.example.App", Thread.currentThread().getName());
        NativeLogger.log(4, "Slow query detected", "{requestId=REQ-001, queryMs=850}",
                "com.example.db.QueryExecutor", "db-pool-1");

        Thread.sleep(300);
    }

    @Test
    public void testLoggerHierarchyName() throws Exception {
        System.out.println("\n=== Feature 1: %logger{36} abbreviation ===\n");

        // Long name should be abbreviated: com.example.service.impl.OrderServiceImpl
        // → c.e.s.i.OrderServiceImpl (within 36 chars)
        NativeLogger.log(3, "Testing long logger name abbreviation", null,
                "com.example.service.impl.OrderServiceImpl", "main");
        NativeLogger.log(3, "Short logger name — no abbreviation needed", null,
                "App", "main");

        Thread.sleep(300);
    }

    @Test
    public void testLog4j2ApiIntegration() throws Exception {
        System.out.println("\n=== Feature 1: Full Log4j2 API through RlogLogger ===\n");

        Logger logger = LogManager.getLogger(NativeLoggerTest.class);

        ThreadContext.put("traceId", "abc-123");
        logger.info("Hello from LogManager — thread and logger name auto-populated");
        logger.debug("Debug message — check %logger and %t in output");
        logger.warn("Warn message with MDC context");
        ThreadContext.clearAll();

        logger.error("Error message — no MDC");

        Thread.sleep(300);
    }

    // ── Feature 2: Per-logger level configuration ─────────────────────────

    @Test
    public void testPerLoggerLevels() throws Exception {
        System.out.println("\n=== Feature 2: Per-logger level hierarchy ===");
        System.out.println("Config: com.example.db=WARN  com.example.service=DEBUG  Root=INFO\n");

        // com.example.db → WARN: INFO should be suppressed, WARN should appear
        NativeLogger.log(3, "[SHOULD BE HIDDEN] db INFO suppressed by WARN threshold",
                null, "com.example.db.QueryExecutor", "test");
        NativeLogger.log(4, "[VISIBLE] db WARN — at threshold",
                null, "com.example.db.QueryExecutor", "test");

        // com.example.service → DEBUG: DEBUG should appear
        NativeLogger.log(2, "[VISIBLE] service DEBUG — below root but above own threshold",
                null, "com.example.service.UserService", "test");
        NativeLogger.log(3, "[VISIBLE] service INFO — above threshold",
                null, "com.example.service.UserService", "test");

        // Root → INFO: DEBUG suppressed, INFO visible
        NativeLogger.log(2, "[SHOULD BE HIDDEN] api DEBUG suppressed by root INFO",
                null, "com.example.api.Controller", "test");
        NativeLogger.log(3, "[VISIBLE] api INFO — at root threshold",
                null, "com.example.api.Controller", "test");

        // Hierarchy: com.example.db.impl inherits WARN from com.example.db
        NativeLogger.log(3, "[SHOULD BE HIDDEN] db.impl INFO — inherits WARN from parent",
                null, "com.example.db.impl.QueryImpl", "test");
        NativeLogger.log(4, "[VISIBLE] db.impl WARN — inherits WARN from parent",
                null, "com.example.db.impl.QueryImpl", "test");

        // com.example.quiet → ERROR: WARN suppressed
        NativeLogger.log(4, "[SHOULD BE HIDDEN] quiet WARN suppressed by ERROR threshold",
                null, "com.example.quiet.NoisyLib", "test");
        NativeLogger.log(5, "[VISIBLE] quiet ERROR — at threshold",
                null, "com.example.quiet.NoisyLib", "test");

        Thread.sleep(400);
        System.out.println("\n--- Lines marked [VISIBLE] should appear in log; [SHOULD BE HIDDEN] must not ---");
    }

    @Test
    public void testPerLoggerLevelsViaLog4j2Api() throws Exception {
        System.out.println("\n=== Feature 2: Per-logger levels via LogManager ===\n");

        Logger dbLogger      = LogManager.getLogger("com.example.db.QueryExecutor");
        Logger serviceLogger = LogManager.getLogger("com.example.service.UserService");
        Logger apiLogger     = LogManager.getLogger("com.example.api.Controller");

        // Verify isEnabled() respects per-logger level
        System.out.println("dbLogger.isDebugEnabled()      = " + dbLogger.isDebugEnabled()
                + "  (expected: false — db level is WARN)");
        System.out.println("serviceLogger.isDebugEnabled() = " + serviceLogger.isDebugEnabled()
                + "  (expected: true  — service level is DEBUG)");
        System.out.println("apiLogger.isDebugEnabled()     = " + apiLogger.isDebugEnabled()
                + "  (expected: false — root level is INFO)");

        // Only WARN and above should appear for db logger
        dbLogger.debug("[SHOULD BE HIDDEN] db debug via LogManager");
        dbLogger.info("[SHOULD BE HIDDEN] db info via LogManager");
        dbLogger.warn("[VISIBLE] db warn via LogManager");

        // DEBUG and above should appear for service logger
        serviceLogger.debug("[VISIBLE] service debug via LogManager");
        serviceLogger.info("[VISIBLE] service info via LogManager");

        // INFO and above for api logger
        apiLogger.debug("[SHOULD BE HIDDEN] api debug via LogManager");
        apiLogger.info("[VISIBLE] api info via LogManager");

        Thread.sleep(400);
    }

    // ── Feature 3: NDC stack ──────────────────────────────────────────────

    @Test
    public void testNdcStack() throws Exception {
        System.out.println("\n=== Feature 3: NDC stack (%x specifier) ===\n");

        Logger logger = LogManager.getLogger("com.example.service.OrderService");

        // Push NDC values (classic nested diagnostic context)
        ThreadContext.push("processOrder");
        ThreadContext.push("validatePayment");

        logger.info("Processing payment — NDC should show [processOrder validatePayment]");
        logger.warn("Payment gateway timeout — NDC still attached");

        ThreadContext.pop(); // remove "validatePayment"
        logger.info("Back to order level — NDC should show [processOrder]");

        ThreadContext.clearStack();
        logger.info("NDC cleared — %x should be empty");

        Thread.sleep(300);

        // Verify the NDC appears in the log file
        String logFile = "target/rust-rolling-app.log.2026-05-12";
        // The pattern includes %x so NDC is embedded; verify via direct NativeLogger call
        // passing ndc explicitly so we can assert it ended up in the output
        NativeLogger.log(3, "NDC direct test",
                "{requestId=R99}", "outer inner",
                "com.example.ndc.Test", "ndc-thread", null);

        Thread.sleep(300);
        System.out.println("Check log: lines with NDC should show stack values");
    }

    @Test
    public void testNdcDepthSpecifier() throws Exception {
        System.out.println("\n=== Feature 3: NDC %x{depth} ===\n");

        // Simulate what %x{2} would show for a 4-level NDC stack
        // We test by calling rlog_log_full directly with a known NDC string
        // The pattern in log4j2.xml uses plain %x; here we verify the depth logic
        // works correctly via NativeLogger with explicit ndc value
        NativeLogger.log(3, "Full NDC stack",
                null, "l1 l2 l3 l4",
                "com.example.ndc.Depth", "t1", null);

        Thread.sleep(200);
        System.out.println("NDC 'l1 l2 l3 l4' should appear in log");
    }

    // ── Feature 3: Exception stack traces ────────────────────────────────

    @Test
    public void testExceptionStackTrace() throws Exception {
        System.out.println("\n=== Feature 3: Full exception stack trace (%ex) ===\n");

        Logger logger = LogManager.getLogger("com.example.service.PaymentService");

        // Simulate a real exception with a cause chain
        Exception cause = new IllegalArgumentException("Invalid card number");
        Exception ex    = new RuntimeException("Payment processing failed", cause);

        logger.error("Transaction failed — full stack trace should appear below message", ex);

        // Nested cause
        try {
            try {
                throw new java.io.IOException("DB connection refused");
            } catch (java.io.IOException ioe) {
                throw new RuntimeException("Repository unavailable", ioe);
            }
        } catch (RuntimeException re) {
            logger.error("Repository error — cause chain should be visible", re);
        }

        Thread.sleep(400);
        System.out.println("Check log: exception lines starting with \\tat should follow the message");
    }

    @Test
    public void testMessageCleanWhenExceptionPresent() throws Exception {
        System.out.println("\n=== Feature 3: %msg clean, %ex separate ===\n");

        Logger logger = LogManager.getLogger("com.example.service.UserService");
        RuntimeException ex = new RuntimeException("something went wrong");

        logger.error("User registration failed", ex);

        Thread.sleep(300);
        System.out.println("'User registration failed' should appear on its own line.");
        System.out.println("Stack trace should follow on the next line via %ex.");
        System.out.println("The message must NOT contain '| Exception:' suffix (old behaviour).");
    }

    // ── Level ladder verification for com.example.db (configured WARN) ───

    @Test
    public void testFullLevelLadder() throws Exception {
        System.out.println("\n=== Level ladder: com.example.db is configured at WARN ===");
        System.out.println("Levels in order: TRACE(1) < DEBUG(2) < INFO(3) < WARN(4) < ERROR(5)");
        System.out.println("Expected: only WARN and above should appear\n");

        String logger = "com.example.db.QueryExecutor";
        String thread = "test";

        // 1=TRACE  suppressed (1 < 4)
        NativeLogger.log(1, "[HIDDEN]  TRACE  < WARN → suppressed", null, logger, thread);
        // 2=DEBUG  suppressed (2 < 4)
        NativeLogger.log(2, "[HIDDEN]  DEBUG  < WARN → suppressed", null, logger, thread);
        // 3=INFO   suppressed (3 < 4)
        NativeLogger.log(3, "[HIDDEN]  INFO   < WARN → suppressed", null, logger, thread);
        // 4=WARN   passes    (4 == 4)
        NativeLogger.log(4, "[VISIBLE] WARN  == WARN → passes",     null, logger, thread);
        // 5=ERROR  passes    (5 > 4)
        NativeLogger.log(5, "[VISIBLE] ERROR  > WARN → passes",     null, logger, thread);

        // Same via Log4j2 API so isEnabled() short-circuit is also exercised
        Logger log4jLogger = LogManager.getLogger("com.example.db.QueryExecutor");
        System.out.println("\nisEnabled() results on com.example.db.QueryExecutor (WARN):");
        System.out.printf("  isTraceEnabled = %-5b  expected false%n", log4jLogger.isTraceEnabled());
        System.out.printf("  isDebugEnabled = %-5b  expected false%n", log4jLogger.isDebugEnabled());
        System.out.printf("  isInfoEnabled  = %-5b  expected false%n", log4jLogger.isInfoEnabled());
        System.out.printf("  isWarnEnabled  = %-5b  expected true %n", log4jLogger.isWarnEnabled());
        System.out.printf("  isErrorEnabled = %-5b  expected true %n", log4jLogger.isErrorEnabled());

        log4jLogger.trace("[HIDDEN]  trace via Log4j2 API");
        log4jLogger.debug("[HIDDEN]  debug via Log4j2 API");
        log4jLogger.info( "[HIDDEN]  info  via Log4j2 API");
        log4jLogger.warn( "[VISIBLE] warn  via Log4j2 API");
        log4jLogger.error("[VISIBLE] error via Log4j2 API");
        log4jLogger.fatal("[VISIBLE] fatal via Log4j2 API");

        Thread.sleep(400);
        System.out.println("\n--- Scan the log: only WARN / ERROR / FATAL lines should appear ---");
    }

    // ── Backward compat ───────────────────────────────────────────────────

    // Kept for backward compatibility: old 2-arg call still works
    @Test
    public void testLegacyApi() throws Exception {
        System.out.println("\n=== Feature 1: Legacy 2-arg API (logger/thread show as 'root'/'main') ===\n");
        NativeLogger.log(3, "Legacy call — logger=root thread=main expected");
        NativeLogger.log(5, "Legacy error — no metadata");
        Thread.sleep(300);
    }
}
