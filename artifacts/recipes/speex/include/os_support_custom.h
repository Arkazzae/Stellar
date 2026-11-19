/*
 * SpeexDSP's os_support.h includes this file when OS_SUPPORT_CUSTOM is
 * defined, and keeps its own version of anything defined here.
 *
 * The library would print warnings and fatal errors to stderr and exit. The
 * binary runs on the audio thread with no imports, so it has neither: a
 * warning is dropped, and a fatal error traps, which the processor sees as an
 * exception from the call. Dropping the file name and line also keeps the
 * build machine's paths, which speex_fatal passes as __FILE__, out of the
 * binary.
 */
#ifndef STELLARR_OS_SUPPORT_CUSTOM_H
#define STELLARR_OS_SUPPORT_CUSTOM_H

#define OVERRIDE_SPEEX_FATAL
static inline void _speex_fatal(const char *str, const char *file, int line)
{
   (void)str;
   (void)file;
   (void)line;
   __builtin_trap();
}

#define OVERRIDE_SPEEX_WARNING
static inline void speex_warning(const char *str)
{
   (void)str;
}

#define OVERRIDE_SPEEX_WARNING_INT
static inline void speex_warning_int(const char *str, int val)
{
   (void)str;
   (void)val;
}

#define OVERRIDE_SPEEX_NOTIFY
static inline void speex_notify(const char *str)
{
   (void)str;
}

#define OVERRIDE_SPEEX_PUTC
static inline void _speex_putc(int ch, void *file)
{
   (void)ch;
   (void)file;
}

#endif
