"""Python wrapper for the flattened config functions in _core."""

import probing._core as _core
from probing._native import call_native


def get(key):
    return call_native(_core.config_get, key)


def set(key, value):
    return call_native(_core.config_set, key, value)


def get_str(key):
    return call_native(_core.config_get_str, key)


def contains_key(key):
    return call_native(_core.config_contains_key, key)


def remove(key):
    return call_native(_core.config_remove, key)


def keys():
    return call_native(_core.config_keys)


def clear():
    return call_native(_core.config_clear)


def len():
    return call_native(_core.config_len)


def is_empty():
    return call_native(_core.config_is_empty)
