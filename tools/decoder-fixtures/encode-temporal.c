// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

#include "avm/avm_encoder.h"
#include "avm/avmcx.h"

static void check(avm_codec_ctx_t *codec, avm_codec_err_t error) {
  if (error == AVM_CODEC_OK) return;

  const char *detail = avm_codec_error_detail(codec);
  fprintf(stderr, "%s\n", detail ? detail : avm_codec_err_to_string(error));
  exit(EXIT_FAILURE);
}

static void little_endian(FILE *output, uint64_t value, int bytes) {
  for (int i = 0; i < bytes; ++i) {
    if (fputc(value & 255, output) == EOF) exit(EXIT_FAILURE);
    value >>= 8;
  }
}

static void fill_image(avm_image_t *image, int frame, unsigned depth,
                       FILE *source) {
  for (int plane = 0; plane < 3; ++plane) {
    unsigned width = image->d_w >> (plane != 0);
    unsigned height = image->d_h >> (plane != 0);
    for (unsigned y = 0; y < height; ++y) {
      uint8_t *row = image->planes[plane] + y * image->stride[plane];
      for (unsigned x = 0; x < width; ++x) {
        unsigned sample = 32 + (3 * x + 5 * y + 7 * frame + 61 * plane) % 192;
        if (depth == 8) {
          row[x] = sample;
          little_endian(source, sample, 1);
        } else {
          sample = (sample << 2) + (x + y + frame) % 4;
          ((uint16_t *)row)[x] = sample;
          little_endian(source, sample, 2);
        }
      }
    }
  }
}

int main(int argc, char **argv) {
  if (argc != 3 || argv[1][0] < '0' || argv[1][0] > '6' || argv[1][1]) {
    fputs("Specify a fixture index from 0 to 6 and a source output path.\n", stderr);
    return EXIT_FAILURE;
  }

  FILE *source = fopen(argv[2], "wb");
  if (!source) {
    perror("Cannot open the source output");
    return EXIT_FAILURE;
  }

  int fixture = argv[1][0] - '0';
  int sparse_temporal = fixture == 2;
  int embedded = fixture >= 3;
  int mixed_resolution = fixture == 5;
  int embedded_temporal = fixture == 6;
  int annex_b = sparse_temporal || embedded;
  unsigned depth = fixture == 1 || fixture == 4 ? 10 : 8;
  unsigned width = mixed_resolution ? 256 : (fixture == 0 || embedded_temporal ? 128 : 64);
  unsigned height = mixed_resolution ? width : 64;
  unsigned frames = embedded ? (embedded_temporal ? 4 : 3) : (sparse_temporal ? 5 : 4);
  unsigned mlayers = embedded ? 2 : 1;
  unsigned tlayers = sparse_temporal ? 4 : embedded && !embedded_temporal ? 1 : 2;
  avm_codec_iface_t *encoder = avm_codec_av2_cx();
  avm_codec_ctx_t codec = {0};
  avm_codec_enc_cfg_t cfg;
  check(&codec, avm_codec_enc_config_default(encoder, &cfg, 0));
  cfg.g_w = width;
  cfg.g_h = height;
  cfg.g_timebase.num = 1;
  cfg.g_timebase.den = 30;
  cfg.g_threads = 1;
  cfg.g_profile = embedded ? (fixture == 3 ? 1 : 2) : 0;
  cfg.g_bit_depth = depth == 8 ? AVM_BITS_8 : AVM_BITS_10;
  cfg.g_input_bit_depth = depth;
  cfg.g_lag_in_frames = 0;
  cfg.enable_ops = embedded_temporal;
  cfg.enable_lcr = embedded;
  cfg.rc_end_usage = AVM_Q;
  cfg.rc_min_quantizer = 120;
  cfg.rc_max_quantizer = 120;

  check(&codec, avm_codec_enc_init(&codec, encoder, &cfg, 0));
  if (embedded_temporal)
    check(&codec, avm_codec_set_option(&codec, "enable-mfh-obu-signaling", "1"));
  if (embedded)
    check(&codec, avm_codec_control(&codec, AVME_SET_NUMBER_MLAYERS, mlayers));
  check(&codec, avm_codec_control(&codec, AVME_SET_NUMBER_TLAYERS, tlayers));
  if (embedded)
    check(&codec, avm_codec_control(&codec, AV2E_SET_ENABLE_FLAG_MULTI_LAYER_LAG_TEST, 1));
  check(&codec, avm_codec_control(&codec, AVME_SET_CPUUSED, 6));
  check(&codec, avm_codec_control(&codec, AV2E_SET_ENABLE_QM, 0));
  if (fixture == 0 || embedded_temporal) {
    check(&codec, avm_codec_control(&codec, AV2E_SET_TILE_COLUMNS, 1));
    check(&codec, avm_codec_control(&codec, AV2E_SET_NUM_TG, 2));
  }

  avm_image_t image;
  if (!avm_img_alloc(&image, depth == 8 ? AVM_IMG_FMT_I420 : AVM_IMG_FMT_I42016,
                     width, height, 32)) {
    fputs("Cannot allocate the input frame.\n", stderr);
    return EXIT_FAILURE;
  }
  if (!annex_b) {
    if (fwrite("DKIF\0\0\x20\0AV02", 1, 12, stdout) != 12)
      return EXIT_FAILURE;
    little_endian(stdout, width, 2);
    little_endian(stdout, height, 2);
    little_endian(stdout, 30, 4);
    little_endian(stdout, 1, 4);
    little_endian(stdout, frames, 4);
    little_endian(stdout, 0, 4);
  }

  unsigned emitted = 0;
  for (unsigned frame = 0; frame <= frames; ++frame) {
    int tlayer = embedded_temporal ? frame != 0 : frame % tlayers;
    if (frame < frames) fill_image(&image, frame, depth, source);
    unsigned calls = frame < frames ? mlayers : 1;
    for (unsigned mlayer = 0; mlayer < calls; ++mlayer) {
      int no_update = (sparse_temporal && (tlayer == 1 || tlayer == 3)) ||
                      (embedded_temporal && mlayer != 0 && tlayer != 0);
      if (frame < frames) {
        if (mixed_resolution) {
          AVM_SCALING_MODE scale = mlayer == 0 ? AVME_ONETWO : AVME_NORMAL;
          struct avm_scaling_mode mode = { scale, scale };
          check(&codec, avm_codec_control(&codec, AVME_SET_SCALEMODE, &mode));
        }
        if (embedded)
          check(&codec, avm_codec_control(&codec, AVME_SET_MLAYER_ID, mlayer));
        check(&codec, avm_codec_control(&codec, AVME_SET_TLAYER_ID, tlayer));
      }
      check(&codec, avm_codec_encode(&codec, frame < frames ? &image : NULL,
                                     frame * mlayers + mlayer, 1,
                                     no_update ? AVM_EFLAG_NO_UPD_ALL : 0));

      avm_codec_iter_t iter = NULL;
      const avm_codec_cx_pkt_t *packet;
      while ((packet = avm_codec_get_cx_data(&codec, &iter))) {
        if (packet->kind != AVM_CODEC_CX_FRAME_PKT) continue;
        if (!annex_b) {
          little_endian(stdout, packet->data.frame.sz, 4);
          little_endian(stdout, packet->data.frame.pts, 8);
        }
        if (fwrite(packet->data.frame.buf, 1, packet->data.frame.sz, stdout) !=
            packet->data.frame.sz)
          return EXIT_FAILURE;
        ++emitted;
      }
    }
  }

  if (fclose(source) != 0) return EXIT_FAILURE;
  avm_img_free(&image);
  check(&codec, avm_codec_destroy(&codec));
  if (emitted != frames * mlayers) {
    fputs("The encoder emitted an unexpected number of frames.\n", stderr);
    return EXIT_FAILURE;
  }
  return fflush(stdout) == 0 ? EXIT_SUCCESS : EXIT_FAILURE;
}
