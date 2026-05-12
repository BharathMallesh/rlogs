package org.apache.logging.log4j.core;

import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;
import org.apache.logging.log4j.Marker;
import org.apache.logging.log4j.MarkerManager;
import org.apache.logging.log4j.ThreadContext;
import org.junit.jupiter.api.Test;

/**
 * Tests for Features 5 – 10.
 *
 *  5 – Additional appenders (Socket, Syslog, HTTP) — connection-failure paths tested
 *  6 – Additional filters (MarkerFilter, BurstFilter, DynamicThresholdFilter)
 *  7 – Additional layouts (HTML, XML, CSV)
 *  8 – Caller location (%C %M %L %F)
 *  9 – Marker support (%marker)
 * 10 – Lookup plugins (${date:…}, ${ctx:…})
 */
public class AdvancedFeaturesTest {

    // ── Feature 5: Additional appenders ──────────────────────────────────────

    @Test
    public void testSocketAppenderConfigParsed() throws Exception {
        System.out.println("\n=== Feature 5: SocketAppender (TCP) config parsing ===\n");

        // Configure with a socket appender targeting localhost:19999 (nothing listening).
        // The appender should log an error on connect and fall back to console.
        String xml = buildXml(
            "<Socket name=\"Socket\" host=\"127.0.0.1\" port=\"19999\"/>\n"
          + "<Console name=\"Console\" target=\"SYSTEM_OUT\">"
          + "<PatternLayout pattern=\"%d{HH:mm:ss.SSS} [%t] %-5level %logger{36} - %msg%n\"/>"
          + "</Console>",
            "", "INFO");
        Configurator.reconfigure(xml);
        Thread.sleep(200);

        Logger logger = LogManager.getLogger("com.example.socket");
        logger.info("[VISIBLE on Console] socket appender unreachable — falls back");
        Thread.sleep(300);
        System.out.println("Socket appender: configured (connection error printed if port closed)");

        Configurator.reconfigure(originalXml());
        Thread.sleep(200);
    }

    @Test
    public void testSyslogAppenderConfigParsed() throws Exception {
        System.out.println("\n=== Feature 5: SyslogAppender (UDP RFC-3164) config parsing ===\n");

        String xml = buildXml(
            "<Syslog name=\"Syslog\" host=\"127.0.0.1\" port=\"19514\" facility=\"USER\"/>\n"
          + "<Console name=\"Console\" target=\"SYSTEM_OUT\">"
          + "<PatternLayout pattern=\"%d{HH:mm:ss.SSS} [%t] %-5level %logger{36} - %msg%n\"/>"
          + "</Console>",
            "", "INFO");
        Configurator.reconfigure(xml);
        Thread.sleep(200);

        Logger logger = LogManager.getLogger("com.example.syslog");
        logger.warn("[UDP packet sent to 127.0.0.1:19514 — dropped silently if nothing listens]");
        Thread.sleep(300);
        System.out.println("Syslog appender: UDP packet fired (no listener needed to test config)");

        Configurator.reconfigure(originalXml());
        Thread.sleep(200);
    }

    @Test
    public void testHttpAppenderConfigParsed() throws Exception {
        System.out.println("\n=== Feature 5: HttpAppender (POST) config parsing ===\n");

        // Use a non-routable address so the POST fails quickly.
        String xml = buildXml(
            "<Http name=\"Http\" url=\"http://127.0.0.1:19999/logs\" method=\"POST\"/>\n"
          + "<Console name=\"Console\" target=\"SYSTEM_OUT\">"
          + "<PatternLayout pattern=\"%d{HH:mm:ss.SSS} [%t] %-5level %logger{36} - %msg%n\"/>"
          + "</Console>",
            "", "INFO");
        Configurator.reconfigure(xml);
        Thread.sleep(200);

        Logger logger = LogManager.getLogger("com.example.http");
        logger.info("[Console line visible; HTTP POST will fail gracefully]");
        Thread.sleep(500);
        System.out.println("HTTP appender: error printed if unreachable (expected)");

        Configurator.reconfigure(originalXml());
        Thread.sleep(200);
    }

    // ── Feature 6: Additional filters ────────────────────────────────────────

    @Test
    public void testMarkerFilter() throws Exception {
        System.out.println("\n=== Feature 6: MarkerFilter ===\n");
        System.out.println("Config: deny events with marker=SENSITIVE\n");

        String xml = buildXml(
            "<Console name=\"Console\" target=\"SYSTEM_OUT\">"
          + "<PatternLayout pattern=\"%d{HH:mm:ss.SSS} [%t] %-5level %logger{36} %marker - %msg%n\"/>"
          + "</Console>",
            "<MarkerFilter marker=\"SENSITIVE\" onMatch=\"DENY\" onMismatch=\"ACCEPT\"/>",
            "INFO");
        Configurator.reconfigure(xml);
        Thread.sleep(200);

        Logger logger = LogManager.getLogger("com.example.marker");
        Marker sensitive = MarkerManager.getMarker("SENSITIVE");
        Marker normal    = MarkerManager.getMarker("AUDIT");

        logger.info(normal,    "[VISIBLE] AUDIT marker — not denied");
        logger.warn(sensitive, "[SHOULD BE HIDDEN] SENSITIVE marker — denied by MarkerFilter");
        logger.info(           "[VISIBLE] no marker — not denied");

        Thread.sleep(300);
        System.out.println("Check: only AUDIT and no-marker lines should appear");

        Configurator.reconfigure(originalXml());
        Thread.sleep(200);
    }

    @Test
    public void testBurstFilter() throws Exception {
        System.out.println("\n=== Feature 6: BurstFilter (rate limit) ===\n");
        System.out.println("Config: max 3 events per 2000ms window\n");

        String xml = buildXml(
            "<Console name=\"Console\" target=\"SYSTEM_OUT\">"
          + "<PatternLayout pattern=\"%d{HH:mm:ss.SSS} [%t] %-5level %logger{36} - %msg%n\"/>"
          + "</Console>",
            "<BurstFilter maxBurst=\"3\" intervalMillis=\"2000\" onMatch=\"DENY\"/>",
            "INFO");
        Configurator.reconfigure(xml);
        Thread.sleep(200);

        Logger logger = LogManager.getLogger("com.example.burst");
        System.out.println("Sending 6 rapid messages — only first 3 should appear:");
        for (int i = 1; i <= 6; i++) {
            logger.info("Burst message #" + i + (i <= 3 ? " [VISIBLE]" : " [SHOULD BE HIDDEN]"));
        }

        Thread.sleep(500);
        System.out.println("Check: messages 4-6 suppressed by burst limit");

        Configurator.reconfigure(originalXml());
        Thread.sleep(200);
    }

    @Test
    public void testDynamicThresholdFilter() throws Exception {
        System.out.println("\n=== Feature 6: DynamicThresholdFilter ===\n");
        System.out.println("Config: MDC key 'logLevel' drives per-event threshold\n");

        String xml = buildXml(
            "<Console name=\"Console\" target=\"SYSTEM_OUT\">"
          + "<PatternLayout pattern=\"%d{HH:mm:ss.SSS} [%t] %-5level %logger{36} %X - %msg%n\"/>"
          + "</Console>",
            "<DynamicThresholdFilter key=\"logLevel\" defaultThreshold=\"INFO\"/>",
            "TRACE");
        Configurator.reconfigure(xml);
        Thread.sleep(200);

        Logger logger = LogManager.getLogger("com.example.dynfilter");

        // User with logLevel=ERROR in MDC → only ERROR passes
        ThreadContext.put("logLevel", "ERROR");
        logger.debug("[SHOULD BE HIDDEN] debug — MDC threshold=ERROR");
        logger.info( "[SHOULD BE HIDDEN] info  — MDC threshold=ERROR");
        logger.error("[VISIBLE] error — at MDC threshold=ERROR");

        // User with logLevel=DEBUG in MDC → DEBUG passes
        ThreadContext.put("logLevel", "DEBUG");
        logger.debug("[VISIBLE] debug — MDC threshold=DEBUG");
        logger.info( "[VISIBLE] info  — MDC threshold=DEBUG");

        // No MDC key → default threshold INFO
        ThreadContext.clearAll();
        logger.debug("[SHOULD BE HIDDEN] debug — no MDC, default threshold=INFO");
        logger.info( "[VISIBLE] info — no MDC, at default threshold=INFO");

        Thread.sleep(400);

        Configurator.reconfigure(originalXml());
        Thread.sleep(200);
    }

    // ── Feature 7: Additional layouts ────────────────────────────────────────

    @Test
    public void testHtmlLayout() throws Exception {
        System.out.println("\n=== Feature 7: HTMLLayout ===\n");

        String xml = buildXml(
            "<Console name=\"Console\" target=\"SYSTEM_OUT\"><HTMLLayout/></Console>",
            "", "INFO");
        Configurator.reconfigure(xml);
        Thread.sleep(200);

        Logger logger = LogManager.getLogger("com.example.html");
        logger.info("HTML layout test — INFO");
        logger.warn("HTML layout test — WARN with <special> &chars;");
        logger.error("HTML layout test — ERROR");

        Thread.sleep(300);
        System.out.println("Check: output should be <tr>...</tr> HTML rows");

        Configurator.reconfigure(originalXml());
        Thread.sleep(200);
    }

    @Test
    public void testXmlLayout() throws Exception {
        System.out.println("\n=== Feature 7: XMLLayout ===\n");

        String xml = buildXml(
            "<Console name=\"Console\" target=\"SYSTEM_OUT\"><XMLLayout/></Console>",
            "", "INFO");
        Configurator.reconfigure(xml);
        Thread.sleep(200);

        Logger logger = LogManager.getLogger("com.example.xmllayout");
        ThreadContext.put("traceId", "abc-123");
        logger.info("XML layout test — INFO with MDC");
        logger.warn("XML layout test — WARN");
        ThreadContext.clearAll();

        Thread.sleep(300);
        System.out.println("Check: output should be log4j:event XML elements");

        Configurator.reconfigure(originalXml());
        Thread.sleep(200);
    }

    @Test
    public void testCsvLayout() throws Exception {
        System.out.println("\n=== Feature 7: CSVLayout ===\n");

        String xml = buildXml(
            "<Console name=\"Console\" target=\"SYSTEM_OUT\">"
          + "<CSVPatternLayout delimiter=\",\"/>"
          + "</Console>",
            "", "INFO");
        Configurator.reconfigure(xml);
        Thread.sleep(200);

        Logger logger = LogManager.getLogger("com.example.csv");
        logger.info("CSV row — INFO");
        logger.warn("CSV with, comma inside message");
        logger.error("CSV row — ERROR");

        Thread.sleep(300);
        System.out.println("Check: output should be comma-separated values (message with comma is quoted)");

        Configurator.reconfigure(originalXml());
        Thread.sleep(200);
    }

    // ── Feature 8: Caller location ───────────────────────────────────────────

    @Test
    public void testCallerLocation() throws Exception {
        System.out.println("\n=== Feature 8: Caller location (%C %M %L %F) ===\n");

        String xml = buildXml(
            "<Console name=\"Console\" target=\"SYSTEM_OUT\">"
          + "<PatternLayout pattern=\"%d{HH:mm:ss.SSS} [%t] %-5level %C.%M(%F:%L) - %msg%n\"/>"
          + "</Console>",
            "", "INFO");
        Configurator.reconfigure(xml);
        Thread.sleep(200);

        Logger logger = LogManager.getLogger("com.example.location");
        logger.info("This line should show class=AdvancedFeaturesTest method=testCallerLocation");
        logger.warn("Warn with caller info");

        Thread.sleep(300);
        System.out.println("Check: class name, method name, file and line number in output");

        Configurator.reconfigure(originalXml());
        Thread.sleep(200);
    }

    // ── Feature 9: Marker support ─────────────────────────────────────────────

    @Test
    public void testMarkerInPattern() throws Exception {
        System.out.println("\n=== Feature 9: %marker specifier in PatternLayout ===\n");

        String xml = buildXml(
            "<Console name=\"Console\" target=\"SYSTEM_OUT\">"
          + "<PatternLayout pattern=\"%d{HH:mm:ss.SSS} [%t] %-5level [%marker] %logger{36} - %msg%n\"/>"
          + "</Console>",
            "", "INFO");
        Configurator.reconfigure(xml);
        Thread.sleep(200);

        Logger logger = LogManager.getLogger("com.example.markers");
        Marker security    = MarkerManager.getMarker("SECURITY");
        Marker performance = MarkerManager.getMarker("PERFORMANCE");

        logger.info(security,    "Login attempt from 192.168.1.1");
        logger.warn(performance, "Slow query: 1200ms");
        logger.info(             "No marker — bracket should be empty");

        Thread.sleep(300);
        System.out.println("Check: [SECURITY], [PERFORMANCE], [] in output");

        Configurator.reconfigure(originalXml());
        Thread.sleep(200);
    }

    // ── Feature 10: Lookup plugins ───────────────────────────────────────────

    @Test
    public void testDateLookup() throws Exception {
        System.out.println("\n=== Feature 10: ${date:yyyy-MM-dd} lookup in config ===\n");

        // The file name in the XML uses a date lookup; we verify it resolves at parse time
        String today = new java.text.SimpleDateFormat("yyyy-MM-dd").format(new java.util.Date());
        String xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"
                + "<Configuration status=\"WARN\">\n"
                + "  <Appenders>\n"
                + "    <Console name=\"Console\" target=\"SYSTEM_OUT\">\n"
                + "      <PatternLayout pattern=\"%d{HH:mm:ss.SSS} [%t] %-5level %logger{36} - %msg%n\"/>\n"
                + "    </Console>\n"
                + "  </Appenders>\n"
                + "  <Loggers>\n"
                + "    <Root level=\"INFO\"><AppenderRef ref=\"Console\"/></Root>\n"
                + "  </Loggers>\n"
                + "</Configuration>";

        // Demonstrate resolution via resolveLookups directly
        String withLookup  = "Log file: ${date:yyyy-MM-dd}/app.log";
        String resolved    = NativeLogger.resolveLookups(withLookup);
        System.out.println("Input:    " + withLookup);
        System.out.println("Resolved: " + resolved);
        System.out.println("Expected: Log file: " + today + "/app.log");

        assert resolved.contains(today) :
            "date lookup must resolve to today's date " + today + " but got: " + resolved;

        Configurator.reconfigure(xml);
        Thread.sleep(200);
        LogManager.getLogger("com.example.lookup").info("date lookup verified: " + resolved);
        Thread.sleep(200);

        Configurator.reconfigure(originalXml());
        Thread.sleep(200);
    }

    @Test
    public void testCtxLookup() throws Exception {
        System.out.println("\n=== Feature 10: ${ctx:key} lookup in config ===\n");

        ThreadContext.put("appVersion", "2.1.0");

        String withLookup = "App version: ${ctx:appVersion:-unknown}";
        String resolved   = NativeLogger.resolveLookups(withLookup);
        System.out.println("Input:    " + withLookup);
        System.out.println("Resolved: " + resolved);

        assert "App version: 2.1.0".equals(resolved) :
            "ctx lookup must return MDC value but got: " + resolved;

        // Verify default value when key is absent
        String missing  = "${ctx:nonexistent:-fallback}";
        String resolved2 = NativeLogger.resolveLookups(missing);
        System.out.println("Missing key resolved to: " + resolved2 + " (expected: fallback)");
        assert "fallback".equals(resolved2);

        ThreadContext.clearAll();
        System.out.println("ctx lookup: PASSED");
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    private static String buildXml(String appendersBody, String filtersBody, String rootLevel) {
        String filters = filtersBody.isEmpty() ? ""
                : "  <Filters>\n    " + filtersBody + "\n  </Filters>\n";
        return "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"
                + "<Configuration status=\"WARN\">\n"
                + filters
                + "  <Appenders>\n    " + appendersBody + "\n  </Appenders>\n"
                + "  <Loggers>\n"
                + "    <Root level=\"" + rootLevel + "\"><AppenderRef ref=\"Console\"/></Root>\n"
                + "  </Loggers>\n"
                + "</Configuration>";
    }

    private static String originalXml() {
        return "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"
                + "<Configuration status=\"WARN\">\n"
                + "  <Appenders>\n"
                + "    <Console name=\"Console\" target=\"SYSTEM_OUT\">\n"
                + "      <PatternLayout pattern=\"%d{HH:mm:ss.SSS} [%t] %-5level %logger{36} - %msg%n%ex\"/>\n"
                + "    </Console>\n"
                + "    <RollingFile name=\"File\" fileName=\"target/rust-rolling-app.log\"\n"
                + "                 filePattern=\"target/rust-rolling-app-%d{yyyy-MM-dd}.log\">\n"
                + "      <PatternLayout pattern=\"%d{yyyy-MM-dd HH:mm:ss} [%t] %-5level %logger{36} - %msg%n%ex\"/>\n"
                + "      <Policies><TimeBasedTriggeringPolicy /></Policies>\n"
                + "    </RollingFile>\n"
                + "  </Appenders>\n"
                + "  <Loggers>\n"
                + "    <Logger name=\"com.example.db\"      level=\"WARN\"  additivity=\"true\"/>\n"
                + "    <Logger name=\"com.example.service\" level=\"DEBUG\" additivity=\"true\"/>\n"
                + "    <Logger name=\"com.example.quiet\"   level=\"ERROR\" additivity=\"true\"/>\n"
                + "    <Root level=\"INFO\">\n"
                + "      <AppenderRef ref=\"Console\"/>\n"
                + "      <AppenderRef ref=\"File\"/>\n"
                + "    </Root>\n"
                + "  </Loggers>\n"
                + "</Configuration>";
    }
}
