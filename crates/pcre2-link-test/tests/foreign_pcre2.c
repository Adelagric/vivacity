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

/* libphp.a also bundles bzip2 (ext/bz2). vivacity's `zip` runs bzip2 on a
   pure-Rust backend whose exports carry their own prefix
   (LIBBZ2_RS_SYS_v0.1.x_BZ2_*); should bzip2-sys ever come back into the
   graph, these become duplicates (or shadow the real ones — caught by the
   round trip in link.rs: BZ_CONFIG_ERROR (-9) from the init functions makes
   the bzip2 crate fail at once rather than loop on a garbage stream). */
int BZ2_bzCompressInit(void) { return -9; }
int BZ2_bzCompress(void) { return -9; }
int BZ2_bzCompressEnd(void) { return -9; }
int BZ2_bzDecompressInit(void) { return -9; }
int BZ2_bzDecompress(void) { return -9; }
int BZ2_bzDecompressEnd(void) { return -9; }
const char *BZ2_bzlibVersion(void) { return "foreign"; }
