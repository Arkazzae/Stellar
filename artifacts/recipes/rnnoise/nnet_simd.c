/*
 * Stands in for RNNoise's src/nnet_default.c, which instantiates the
 * network's layer code (nnet_arch.h) with the generic kernels of vec.h. The
 * library has AVX and NEON kernels and nothing for WebAssembly, so the int8
 * matrix products, nearly all of the network's work, would run one multiply
 * at a time. This keeps vec.h and its generic kernels and replaces the two
 * int8 ones with WebAssembly SIMD versions before nnet_arch.h uses them.
 *
 * They give the same bits as the generic kernels. Each block of the weight
 * matrix is 8 rows by 4 columns of int8, and the generic code adds a row's
 * four products as integers, converts that sum to float and adds it to the
 * row's float total, block after block. Here the four products of 8 rows
 * come from four i32x4.dot_i16x8_s and two pairwise sums, exact in 32 bits,
 * and are converted and added to the totals in the same order. In cgemv8x4
 * the generic code multiplies in float, but every product and sum is an
 * integer below 2^24, so float holds it exactly and the result is the same.
 */
#include <math.h>
#include <wasm_simd128.h>

/* vec.h's own kernels, under other names, unused. */
#define sparse_cgemv8x4 generic_sparse_cgemv8x4
#define cgemv8x4 generic_cgemv8x4
#include "vec.h"
#undef sparse_cgemv8x4
#undef cgemv8x4

/* The int32 sums of one block's 8 rows, rows 0-3 and 4-7, from 32 weights and 4 inputs. */
static inline void block_sums(const opus_int8 *w, const opus_int8 *x, v128_t *rows03, v128_t *rows47)
{
   /* x0 x1 x2 x3 twice, as int16. */
   v128_t xx = wasm_i16x8_extend_low_i8x16(wasm_v128_load32_splat(x));
   v128_t w03 = wasm_v128_load(w);
   v128_t w47 = wasm_v128_load(w + 16);
   /* Each lane holds two of a row's four products: rows 0 and 1, then 2 and 3, ... */
   v128_t d01 = wasm_i32x4_dot_i16x8(wasm_i16x8_extend_low_i8x16(w03), xx);
   v128_t d23 = wasm_i32x4_dot_i16x8(wasm_i16x8_extend_high_i8x16(w03), xx);
   v128_t d45 = wasm_i32x4_dot_i16x8(wasm_i16x8_extend_low_i8x16(w47), xx);
   v128_t d67 = wasm_i32x4_dot_i16x8(wasm_i16x8_extend_high_i8x16(w47), xx);
   *rows03 = wasm_i32x4_add(wasm_i32x4_shuffle(d01, d23, 0, 2, 4, 6), wasm_i32x4_shuffle(d01, d23, 1, 3, 5, 7));
   *rows47 = wasm_i32x4_add(wasm_i32x4_shuffle(d45, d67, 0, 2, 4, 6), wasm_i32x4_shuffle(d45, d67, 1, 3, 5, 7));
}

static inline void sparse_cgemv8x4(float *out, const opus_int8 *w, const int *idx, const float *scale, int rows, int cols, const float *_x)
{
   int i, j;
   opus_int8 x[MAX_INPUTS];
   for (i=0;i<cols;i++) x[i] = (int)floor(.5+127*_x[i]);
   for (i=0;i<rows;i+=8)
   {
      int colblocks;
      v128_t y03 = wasm_f32x4_splat(0);
      v128_t y47 = wasm_f32x4_splat(0);
      colblocks = *idx++;
      for (j=0;j<colblocks;j++)
      {
         v128_t s03, s47;
         block_sums(w, &x[*idx++], &s03, &s47);
         y03 = wasm_f32x4_add(y03, wasm_f32x4_convert_i32x4(s03));
         y47 = wasm_f32x4_add(y47, wasm_f32x4_convert_i32x4(s47));
         w += 32;
      }
      wasm_v128_store(&out[i], y03);
      wasm_v128_store(&out[i+4], y47);
   }
   for (i=0;i<rows;i++) out[i] *= scale[i];
}

static inline void cgemv8x4(float *out, const opus_int8 *w, const float *scale, int rows, int cols, const float *_x)
{
   int i, j;
   opus_int8 x[MAX_INPUTS];
   for (i=0;i<cols;i++) x[i] = (int)floor(.5+127*_x[i]);
   for (i=0;i<rows;i+=8)
   {
      v128_t y03 = wasm_f32x4_splat(0);
      v128_t y47 = wasm_f32x4_splat(0);
      for (j=0;j<cols;j+=4)
      {
         v128_t s03, s47;
         block_sums(w, &x[j], &s03, &s47);
         y03 = wasm_f32x4_add(y03, wasm_f32x4_convert_i32x4(s03));
         y47 = wasm_f32x4_add(y47, wasm_f32x4_convert_i32x4(s47));
         w += 32;
      }
      wasm_v128_store(&out[i], y03);
      wasm_v128_store(&out[i+4], y47);
   }
   for (i=0;i<rows;i++) out[i] *= scale[i];
}

#define RTCD_ARCH c
#include "nnet_arch.h"
