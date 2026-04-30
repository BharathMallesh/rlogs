package org.apache.logging.log4j.core;

import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.Locale;

public class NativeLoader {
    private static boolean loaded = false;

    public static synchronized void load() {
        if (loaded) return;

        try {
            // First, try loading from java.library.path
            System.loadLibrary("rlog_core");
            loaded = true;
            return;
        } catch (UnsatisfiedLinkError e) {
            // Fallback to extracting from JAR
            try {
                String os = getOSName();
                String arch = getArchName();
                String libName = System.mapLibraryName("rlog_core");
                
                // macOS adds .dylib, Linux .so, Windows .dll
                String resourcePath = "/META-INF/native/" + os + "/" + arch + "/" + libName;
                
                InputStream is = NativeLoader.class.getResourceAsStream(resourcePath);
                if (is == null) {
                    throw new UnsatisfiedLinkError("Native library not found in JAR at " + resourcePath);
                }

                Path tempDir = Files.createTempDirectory("rlog4-native-");
                Path tempFile = tempDir.resolve(libName);
                
                // Delete on exit
                tempFile.toFile().deleteOnExit();
                tempDir.toFile().deleteOnExit();

                Files.copy(is, tempFile, StandardCopyOption.REPLACE_EXISTING);
                
                System.load(tempFile.toAbsolutePath().toString());
                loaded = true;
            } catch (Exception ex) {
                throw new RuntimeException("Failed to load native library", ex);
            }
        }
    }

    private static String getOSName() {
        String os = System.getProperty("os.name").toLowerCase(Locale.ENGLISH);
        if (os.contains("mac") || os.contains("darwin")) {
            return "macos";
        } else if (os.contains("win")) {
            return "windows";
        } else if (os.contains("nix") || os.contains("nux") || os.contains("aix")) {
            return "linux";
        }
        return "unknown";
    }

    private static String getArchName() {
        String arch = System.getProperty("os.arch").toLowerCase(Locale.ENGLISH);
        if (arch.contains("amd64") || arch.contains("x86_64")) {
            return "x86_64";
        } else if (arch.contains("aarch64") || arch.contains("arm64")) {
            // Java usually uses aarch64 on mac for Apple Silicon
            return "aarch64"; 
        } else if (arch.contains("arm") || arch.contains("aarch32")) {
            return "arm";
        } else if (arch.contains("x86") || arch.contains("i386") || arch.contains("i486") || arch.contains("i586") || arch.contains("i686")) {
            return "x86";
        }
        return arch;
    }
}
