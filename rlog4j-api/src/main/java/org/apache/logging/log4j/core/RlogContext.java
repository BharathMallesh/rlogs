package org.apache.logging.log4j.core;

import org.apache.logging.log4j.message.MessageFactory;
import org.apache.logging.log4j.spi.ExtendedLogger;
import org.apache.logging.log4j.spi.LoggerContext;
import org.apache.logging.log4j.spi.LoggerRegistry;

public class RlogContext implements LoggerContext {

    private final LoggerRegistry<ExtendedLogger> registry = new LoggerRegistry<>();

    @Override
    public Object getExternalContext() { return null; }

    @Override
    public ExtendedLogger getLogger(String name) {
        return getLogger(name, null);
    }

    @Override
    public ExtendedLogger getLogger(String name, MessageFactory messageFactory) {
        if (registry.hasLogger(name, messageFactory)) {
            return registry.getLogger(name, messageFactory);
        }
        ExtendedLogger logger = new RlogLogger(name, messageFactory);
        registry.putIfAbsent(name, messageFactory, logger);
        return registry.getLogger(name, messageFactory);
    }

    @Override
    public boolean hasLogger(String name) {
        return registry.hasLogger(name);
    }

    @Override
    public boolean hasLogger(String name, MessageFactory messageFactory) {
        return registry.hasLogger(name, messageFactory);
    }

    @Override
    public boolean hasLogger(String name, Class<? extends MessageFactory> messageFactoryClass) {
        return registry.hasLogger(name, messageFactoryClass);
    }
}
