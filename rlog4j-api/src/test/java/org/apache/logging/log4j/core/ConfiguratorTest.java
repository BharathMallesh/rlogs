package org.apache.logging.log4j.core;

import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.io.File;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;

import static org.junit.jupiter.api.Assertions.*;

public class ConfiguratorTest {

    // ── Feature 4: runtime reconfiguration via XML string ─────────────────

    @Test
    public void testReconfigureFromXmlString() throws Exception {
        System.out.println("\n=== Feature 4: reconfigure() from XML string ===\n");

        Logger logger = LogManager.getLogger("com.example.reconfig.Service");

        // Baseline — root level is INFO, so DEBUG should be suppressed
        logger.debug("[SHOULD BE HIDDEN] debug before reconfigure (root=INFO)");
        logger.info("[VISIBLE] info before reconfigure");
        Thread.sleep(200);

        // Reconfigure root to DEBUG — DEBUG should now appear
        String debugXml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"
                + "<Configuration status=\"WARN\">\n"
                + "  <Appenders>\n"
                + "    <Console name=\"Console\" target=\"SYSTEM_OUT\">\n"
                + "      <PatternLayout pattern=\"%d{HH:mm:ss.SSS} [%t] %-5level %logger{36} - %msg%n%ex\"/>\n"
                + "    </Console>\n"
                + "  </Appenders>\n"
                + "  <Loggers>\n"
                + "    <Root level=\"DEBUG\">\n"
                + "      <AppenderRef ref=\"Console\"/>\n"
                + "    </Root>\n"
                + "  </Loggers>\n"
                + "</Configuration>";

        Configurator.reconfigure(debugXml);
        Thread.sleep(200);

        System.out.println("--- After reconfigure to DEBUG ---");
        logger.debug("[VISIBLE] debug after reconfigure to DEBUG");
        logger.info("[VISIBLE] info after reconfigure to DEBUG");
        Thread.sleep(300);

        // Verify Java-side isEnabled() also updated
        boolean debugEnabled = logger.isDebugEnabled();
        System.out.println("isDebugEnabled() = " + debugEnabled + "  (expected: true)");
        assertTrue(debugEnabled, "After reconfigure to DEBUG, isDebugEnabled() must be true");

        // Restore original config
        Configurator.reconfigure(originalXml());
        Thread.sleep(200);
    }

    // ── Feature 4: reconfigure from file ──────────────────────────────────

    @Test
    public void testReconfigureFromFile(@TempDir Path tempDir) throws Exception {
        System.out.println("\n=== Feature 4: reconfigure() from file path ===\n");

        File configFile = tempDir.resolve("log4j2-runtime.xml").toFile();
        Files.writeString(configFile.toPath(), warnXml());

        Configurator.reconfigureFromFile(configFile.getAbsolutePath());
        Thread.sleep(200);

        Logger dbLogger = LogManager.getLogger("com.example.db.QueryExecutor");
        System.out.println("isInfoEnabled() = " + dbLogger.isInfoEnabled() + "  (expected: false — root=WARN)");
        assertFalse(dbLogger.isInfoEnabled(), "After reconfigure to WARN root, isInfoEnabled() must be false");

        dbLogger.info("[SHOULD BE HIDDEN] info suppressed after reconfigure to WARN");
        dbLogger.warn("[VISIBLE] warn passes after reconfigure to WARN");
        Thread.sleep(300);

        // Restore
        Configurator.reconfigure(originalXml());
        Thread.sleep(200);
    }

    // ── Feature 4: file-watcher auto-reload ───────────────────────────────

    @Test
    public void testFileWatcherAutoReload(@TempDir Path tempDir) throws Exception {
        System.out.println("\n=== Feature 4: file-watcher auto-reload ===\n");

        File configFile = tempDir.resolve("log4j2-watch.xml").toFile();
        Files.writeString(configFile.toPath(), originalXml());

        Configurator.watchFile(configFile.getAbsolutePath());
        Thread.sleep(500); // let watcher register

        Logger logger = LogManager.getLogger("com.example.watcher.Service");

        // Before file update — root=INFO, debug suppressed
        logger.debug("[SHOULD BE HIDDEN] debug before file update");
        logger.info("[VISIBLE] info before file update");
        Thread.sleep(300);

        System.out.println("--- Modifying watched config file to DEBUG ---");
        // Atomically replace file content (triggers ENTRY_MODIFY)
        Files.writeString(configFile.toPath(), debugXml(), StandardOpenOption.TRUNCATE_EXISTING);

        // macOS WatchService polls every ~10 s; wait up to 15 s for the reload
        long deadline = System.currentTimeMillis() + 15_000;
        while (!logger.isDebugEnabled() && System.currentTimeMillis() < deadline) {
            Thread.sleep(300);
        }

        System.out.println("--- After file-watcher reload ---");
        logger.debug("[VISIBLE] debug after auto-reload to DEBUG");
        logger.info("[VISIBLE] info after auto-reload to DEBUG");
        Thread.sleep(300);

        boolean debugEnabled = logger.isDebugEnabled();
        System.out.println("isDebugEnabled() = " + debugEnabled + "  (expected: true after auto-reload)");
        assertTrue(debugEnabled, "File watcher must have reloaded config to DEBUG");

        Configurator.stopWatching();

        // Restore
        Configurator.reconfigure(originalXml());
        Thread.sleep(200);
    }

    // ── Feature 4: per-logger level change at runtime ─────────────────────

    @Test
    public void testRuntimePerLoggerLevelChange() throws Exception {
        System.out.println("\n=== Feature 4: per-logger level change at runtime ===\n");

        Logger svc = LogManager.getLogger("com.example.service.UserService");

        // Original config: com.example.service = DEBUG
        assertTrue(svc.isDebugEnabled(), "service logger starts at DEBUG");
        svc.debug("[VISIBLE] service debug — initial config");
        Thread.sleep(200);

        // Reconfigure: tighten service to WARN
        String warnSvcXml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"
                + "<Configuration status=\"WARN\">\n"
                + "  <Appenders>\n"
                + "    <Console name=\"Console\" target=\"SYSTEM_OUT\">\n"
                + "      <PatternLayout pattern=\"%d{HH:mm:ss.SSS} [%t] %-5level %logger{36} - %msg%n%ex\"/>\n"
                + "    </Console>\n"
                + "  </Appenders>\n"
                + "  <Loggers>\n"
                + "    <Logger name=\"com.example.service\" level=\"WARN\" additivity=\"true\"/>\n"
                + "    <Root level=\"INFO\">\n"
                + "      <AppenderRef ref=\"Console\"/>\n"
                + "    </Root>\n"
                + "  </Loggers>\n"
                + "</Configuration>";

        Configurator.reconfigure(warnSvcXml);
        Thread.sleep(200);

        System.out.println("isDebugEnabled() = " + svc.isDebugEnabled() + "  (expected: false — tightened to WARN)");
        assertFalse(svc.isDebugEnabled(), "After tightening service to WARN, debug must be disabled");

        svc.debug("[SHOULD BE HIDDEN] service debug after tighten");
        svc.warn("[VISIBLE] service warn after tighten");
        Thread.sleep(300);

        // Restore
        Configurator.reconfigure(originalXml());
        Thread.sleep(200);
    }

    // ── Helpers ───────────────────────────────────────────────────────────

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

    private static String warnXml() {
        return "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"
                + "<Configuration status=\"WARN\">\n"
                + "  <Appenders>\n"
                + "    <Console name=\"Console\" target=\"SYSTEM_OUT\">\n"
                + "      <PatternLayout pattern=\"%d{HH:mm:ss.SSS} [%t] %-5level %logger{36} - %msg%n%ex\"/>\n"
                + "    </Console>\n"
                + "  </Appenders>\n"
                + "  <Loggers>\n"
                + "    <Root level=\"WARN\">\n"
                + "      <AppenderRef ref=\"Console\"/>\n"
                + "    </Root>\n"
                + "  </Loggers>\n"
                + "</Configuration>";
    }

    private static String debugXml() {
        return "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"
                + "<Configuration status=\"WARN\">\n"
                + "  <Appenders>\n"
                + "    <Console name=\"Console\" target=\"SYSTEM_OUT\">\n"
                + "      <PatternLayout pattern=\"%d{HH:mm:ss.SSS} [%t] %-5level %logger{36} - %msg%n%ex\"/>\n"
                + "    </Console>\n"
                + "  </Appenders>\n"
                + "  <Loggers>\n"
                + "    <Root level=\"DEBUG\">\n"
                + "      <AppenderRef ref=\"Console\"/>\n"
                + "    </Root>\n"
                + "  </Loggers>\n"
                + "</Configuration>";
    }
}
