/*
 * The preprocessor asks the echo canceller for its residual echo only when
 * it was given one with SPEEX_PREPROCESS_SET_ECHO_STATE. Nothing here gives
 * it one, so the canceller (mdf.c) is left out of the binary and this stands
 * in for the one function the preprocessor names. Reaching it would mean a
 * caller set an echo state the binary cannot have, so it traps.
 */
struct SpeexEchoState_;

void speex_echo_get_residual(struct SpeexEchoState_ *st, float *residual_echo, int len)
{
   (void)st;
   (void)residual_echo;
   (void)len;
   __builtin_trap();
}
