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
