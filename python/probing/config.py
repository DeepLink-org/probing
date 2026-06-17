"""Python wrapper for the flattened config functions in _core."""

import probing._core as _core


def get(key):
    return _core.config_get(key)


def set(key, value):
    return _core.config_set(key, value)


def get_str(key):
    return _core.config_get_str(key)


def contains_key(key):
    return _core.config_contains_key(key)


def remove(key):
    return _core.config_remove(key)


def keys():
    return _core.config_keys()


def clear():
    return _core.config_clear()


def len():
    return _core.config_len()


def is_empty():
    return _core.config_is_empty()
