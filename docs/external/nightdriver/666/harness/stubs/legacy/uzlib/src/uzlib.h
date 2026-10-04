// Host stub: decompression is not exercised by the framing cases; any compressed packet is rejected by the stub.
#pragma once
#include <cstdint>
#define TINF_DONE 1
struct uzlib_uncomp { const unsigned char* source; const unsigned char* source_limit; int (*source_read_cb)(struct uzlib_uncomp*); unsigned char* dest_start; unsigned char* dest; unsigned char* dest_limit; };
void uzlib_uncompress_init(struct uzlib_uncomp*, void*, unsigned);
int uzlib_zlib_parse_header(struct uzlib_uncomp*);
int uzlib_uncompress_chksum(struct uzlib_uncomp*);
