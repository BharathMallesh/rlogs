package org.apache.logging.log4j.core;

/**
 * Public API for runtime reconfiguration of the Rlog4 native logger.
 *
 * Usage:
 *   // Reload from an XML string (e.g. fetched from a config server)
 *   Configurator.reconfigure(xmlString);
 *
 *   // Reload from a file on disk immediately
 *   Configurator.reconfigureFromFile("/etc/myapp/log4j2.xml");
 *
 *   // Watch a file and reload automatically on every save
 *   Configurator.watchFile("/etc/myapp/log4j2.xml");
 *
 *   // Stop watching
 *   Configurator.stopWatching();
 */
public final class Configurator {

    private Configurator() {}

    /** Apply a new configuration from an XML string immediately. */
    public static void reconfigure(String xmlContent) {
        NativeLogger.reconfigureFromXml(xmlContent);
    }

    /** Read an XML file from disk and apply it immediately. */
    public static void reconfigureFromFile(String configFilePath) {
        NativeLogger.reconfigureFromFile(configFilePath);
    }

    /**
     * Watch a config file and automatically reload whenever it is modified.
     * The watcher runs as a daemon thread so it does not prevent JVM exit.
     */
    public static void watchFile(String configFilePath) {
        NativeLogger.startFileWatcher(configFilePath);
    }

    /** Stop the file-watcher daemon thread if it is running. */
    public static void stopWatching() {
        NativeLogger.stopFileWatcher();
    }
}
