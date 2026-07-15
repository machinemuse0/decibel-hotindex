#pragma once

#include <machine/endian.h>

#ifndef __LITTLE_ENDIAN
#define __LITTLE_ENDIAN LITTLE_ENDIAN
#endif

#ifndef __BIG_ENDIAN
#define __BIG_ENDIAN BIG_ENDIAN
#endif

#ifndef __PDP_ENDIAN
#define __PDP_ENDIAN 3412
#endif

#ifndef __BYTE_ORDER
#define __BYTE_ORDER BYTE_ORDER
#endif

#ifndef __bswap_16
#define __bswap_16(x) __builtin_bswap16(x)
#endif

#ifndef __bswap_32
#define __bswap_32(x) __builtin_bswap32(x)
#endif

#ifndef __bswap_64
#define __bswap_64(x) __builtin_bswap64(x)
#endif

#ifndef __always_inline
#define __always_inline inline __attribute__((always_inline))
#endif
