import sys
import traceback as tb


def crash_handler(type, value, traceback):
    print("=============== Crash Handler ===============", file=sys.stderr)
    tb.print_exception(type, value, traceback, file=sys.stderr)
    print("=============== End Crash Handler ===============", file=sys.stderr)
