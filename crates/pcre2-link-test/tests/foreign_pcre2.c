/* A foreign PCRE2 in miniature: the symbol names PHP's bundled copy exports
   (ext/pcre/pcre2lib, PCRE2_CODE_UNIT_WIDTH=8). `pcre2_compile_8` and
   `_pcre2_check_escape_8` live in pcre2_compile.o, which any use of the
   library pulls in (pcre2_code_free_8 is called from Drop): without the
   prefix they are duplicates at link time. `pcre2_match_8` sits alone in
   pcre2_match.o: a dummy of it would silently shadow the real one instead,
   so the test also checks that a match really matches. */
void *pcre2_compile_8(void) { return 0; }
int _pcre2_check_escape_8(void) { return 0; }
int pcre2_match_8(void) { return -1; }
