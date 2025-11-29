import probing
import probing._core as _core

class Config:
    """Python wrapper for the flattened config functions in _core."""
    
    @staticmethod
    def get(key):
        return _core.config_get(key)
    
    @staticmethod
    def set(key, value):
        return _core.config_set(key, value)
    
    @staticmethod
    def get_str(key):
        return _core.config_get_str(key)
    
    @staticmethod
    def contains_key(key):
        return _core.config_contains_key(key)
    
    @staticmethod
    def remove(key):
        return _core.config_remove(key)
    
    @staticmethod
    def keys():
        return _core.config_keys()
    
    @staticmethod
    def clear():
        return _core.config_clear()
    
    @staticmethod
    def len():
        return _core.config_len()
    
    @staticmethod
    def is_empty():
        return _core.config_is_empty()

# Expose the singleton-like class as the module interface
# This matches the previous behavior where config was a module
import sys
sys.modules[__name__] = Config

