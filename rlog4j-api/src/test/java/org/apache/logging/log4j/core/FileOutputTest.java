package org.apache.logging.log4j.core;

import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;
import org.apache.logging.log4j.ThreadContext;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.io.File;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

import static org.junit.jupiter.api.Assertions.*;

/**
 * Demonstrates writing log output to a plain text file and verifies the
 * content end-to-end (configure → log → flush → read → assert).
 */
public class FileOutputTest {

    @Test
    public void testWriteToTextFile(@TempDir Path tempDir) throws Exception {
        System.out.println("\n=== Writing log output to a text file ===\n");

        File logFile = tempDir.resolve("my-application.log").toFile();
        System.out.println("Target log file: " + logFile.getAbsolutePath());

        // Configure a File appender pointing to our text file
        String xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"
                + "<Configuration status=\"WARN\">\n"
                + "  <Appenders>\n"
                + "    <File name=\"MyFile\" fileName=\"" + logFile.getAbsolutePath() + "\">\n"
                + "      <PatternLayout pattern=\"%d{yyyy-MM-dd HH:mm:ss.SSS} [%t] %-5level %logger{36} - %msg%n%ex\"/>\n"
                + "    </File>\n"
                + "  </Appenders>\n"
                + "  <Loggers>\n"
                + "    <Root level=\"INFO\">\n"
                + "      <AppenderRef ref=\"MyFile\"/>\n"
                + "    </Root>\n"
                + "  </Loggers>\n"
                + "</Configuration>";

        Configurator.reconfigure(xml);
        Thread.sleep(200);

        // Write log lines through the normal Log4j2 API
        Logger logger = LogManager.getLogger("com.example.app.UserService");

        logger.info("User registration started");
        logger.warn("Password complexity warning for user id=42");

        ThreadContext.put("requestId", "REQ-XYZ-001");
        logger.info("Processing payment for order #1234");
        ThreadContext.clearAll();

        try {
            throw new RuntimeException("Simulated DB timeout");
        } catch (RuntimeException ex) {
            logger.error("Database operation failed", ex);
        }

        logger.info("User registration completed");

        // Flush the async queue so all lines are on disk before we read
        Thread.sleep(500);

        // Read the file back and print its contents
        assertTrue(logFile.exists(), "Log file should exist after writing");
        List<String> lines = Files.readAllLines(logFile.toPath());

        System.out.println("\n--- Contents of " + logFile.getName() + " ---");
        for (String line : lines) {
            System.out.println(line);
        }
        System.out.println("--- end of file (" + lines.size() + " lines, " + logFile.length() + " bytes) ---\n");

        // Verify the file contains what we wrote
        String allContent = String.join("\n", lines);

        assertTrue(allContent.contains("User registration started"),
                "File must contain the first INFO line");
        assertTrue(allContent.contains("Password complexity warning for user id=42"),
                "File must contain the WARN line");
        assertTrue(allContent.contains("Processing payment for order #1234"),
                "File must contain the payment INFO line");
        assertTrue(allContent.contains("Database operation failed"),
                "File must contain the ERROR line");
        assertTrue(allContent.contains("Simulated DB timeout"),
                "File must contain the exception message via %ex");
        assertTrue(allContent.contains("RuntimeException"),
                "File must contain the exception class name via %ex");
        assertTrue(allContent.contains("com.example.app.UserService"),
                "File must contain the logger name via %logger{36}");
        assertTrue(allContent.contains("INFO"),
                "File must show INFO level via %-5level");
        assertTrue(allContent.contains("WARN"),
                "File must show WARN level via %-5level");
        assertTrue(allContent.contains("ERROR"),
                "File must show ERROR level via %-5level");

        System.out.println("✅ All assertions passed — file output is working end-to-end");

        // Restore original config so other tests aren't affected
        Configurator.reconfigure(originalXml());
        Thread.sleep(200);
    }

    @Test
    public void testWriteToFileWithCustomPattern(@TempDir Path tempDir) throws Exception {
        System.out.println("\n=== Writing to file with a CUSTOM pattern ===\n");

        File logFile = tempDir.resolve("custom-pattern.log").toFile();

        // Very compact pattern with caller location
        String xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"
                + "<Configuration status=\"WARN\">\n"
                + "  <Appenders>\n"
                + "    <File name=\"MyFile\" fileName=\"" + logFile.getAbsolutePath() + "\">\n"
                + "      <PatternLayout pattern=\"[%level] %C.%M:%L | %msg%n\"/>\n"
                + "    </File>\n"
                + "  </Appenders>\n"
                + "  <Loggers>\n"
                + "    <Root level=\"DEBUG\">\n"
                + "      <AppenderRef ref=\"MyFile\"/>\n"
                + "    </Root>\n"
                + "  </Loggers>\n"
                + "</Configuration>";

        Configurator.reconfigure(xml);
        Thread.sleep(200);

        Logger logger = LogManager.getLogger("com.example.compact");
        logger.debug("debug line written from compact format");
        logger.info("info line — should show caller location");
        logger.warn("warning line");
        logger.error("error line");

        Thread.sleep(400);

        List<String> lines = Files.readAllLines(logFile.toPath());
        System.out.println("--- Contents of " + logFile.getName() + " ---");
        for (String line : lines) System.out.println(line);
        System.out.println("--- end of file (" + lines.size() + " lines) ---\n");

        assertEquals(4, lines.size(), "Should have exactly 4 lines (DEBUG, INFO, WARN, ERROR)");
        assertTrue(lines.get(0).startsWith("[DEBUG]"));
        assertTrue(lines.get(1).startsWith("[INFO]"));
        assertTrue(lines.get(2).startsWith("[WARN]"));
        assertTrue(lines.get(3).startsWith("[ERROR]"));

        assertTrue(lines.get(0).contains("FileOutputTest"), "Caller class should be in line");
        assertTrue(lines.get(0).contains("testWriteToFileWithCustomPattern"), "Caller method should be in line");

        System.out.println("✅ Custom pattern with caller location written correctly to file");

        Configurator.reconfigure(originalXml());
        Thread.sleep(200);
    }

    @Test
    public void testFileAndConsoleSimultaneously(@TempDir Path tempDir) throws Exception {
        System.out.println("\n=== Writing to BOTH console AND file at once ===\n");

        File logFile = tempDir.resolve("dual-output.log").toFile();

        String xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"
                + "<Configuration status=\"WARN\">\n"
                + "  <Appenders>\n"
                + "    <Console name=\"Console\" target=\"SYSTEM_OUT\">\n"
                + "      <PatternLayout pattern=\"[CONSOLE] %d{HH:mm:ss} %-5level - %msg%n\"/>\n"
                + "    </Console>\n"
                + "    <File name=\"MyFile\" fileName=\"" + logFile.getAbsolutePath() + "\">\n"
                + "      <PatternLayout pattern=\"[FILE]    %d{HH:mm:ss} %-5level - %msg%n\"/>\n"
                + "    </File>\n"
                + "  </Appenders>\n"
                + "  <Loggers>\n"
                + "    <Root level=\"INFO\">\n"
                + "      <AppenderRef ref=\"Console\"/>\n"
                + "      <AppenderRef ref=\"MyFile\"/>\n"
                + "    </Root>\n"
                + "  </Loggers>\n"
                + "</Configuration>";

        Configurator.reconfigure(xml);
        Thread.sleep(200);

        Logger logger = LogManager.getLogger("com.example.dual");
        logger.info("This line should appear on BOTH console and in the file");
        logger.warn("This warning too");

        Thread.sleep(400);

        List<String> lines = Files.readAllLines(logFile.toPath());
        System.out.println("\n--- File contents ---");
        for (String line : lines) System.out.println(line);
        System.out.println("--- end of file ---\n");

        assertEquals(2, lines.size());
        assertTrue(lines.get(0).contains("This line should appear on BOTH"));
        assertTrue(lines.get(1).contains("This warning too"));

        System.out.println("✅ Same log lines appear on console AND in file");

        Configurator.reconfigure(originalXml());
        Thread.sleep(200);
    }

    private static String originalXml() {
        return "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"
                + "<Configuration status=\"WARN\">\n"
                + "  <Appenders>\n"
                + "    <Console name=\"Console\" target=\"SYSTEM_OUT\">\n"
                + "      <PatternLayout pattern=\"%d{HH:mm:ss.SSS} [%t] %-5level %logger{36} - %msg%n%ex\"/>\n"
                + "    </Console>\n"
                + "  </Appenders>\n"
                + "  <Loggers>\n"
                + "    <Root level=\"INFO\"><AppenderRef ref=\"Console\"/></Root>\n"
                + "  </Loggers>\n"
                + "</Configuration>";
    }
}
