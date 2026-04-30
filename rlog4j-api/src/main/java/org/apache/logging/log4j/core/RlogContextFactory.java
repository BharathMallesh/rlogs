package org.apache.logging.log4j.core;

import java.net.URI;
import org.apache.logging.log4j.spi.LoggerContext;
import org.apache.logging.log4j.spi.LoggerContextFactory;

public class RlogContextFactory implements LoggerContextFactory {

    private static final LoggerContext context = new RlogContext();

    @Override
    public LoggerContext getContext(String fqcn, ClassLoader loader, Object externalContext, boolean currentContext) {
        return context;
    }

    @Override
    public LoggerContext getContext(String fqcn, ClassLoader loader, Object externalContext, boolean currentContext, URI configLocation, String name) {
        return context;
    }

    @Override
    public void removeContext(LoggerContext context) {}

    @Override
    public boolean isClassLoaderDependent() {
        return false;
    }
}
