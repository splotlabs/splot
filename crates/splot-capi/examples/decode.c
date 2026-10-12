/* SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0 */
/* SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com> */

/* Decodes an IVF file through the splot C ABI and writes the visible samples
 * of every picture to a raw file: Y, U, V rows, 8-bit samples as bytes,
 * 10-bit samples as little-endian uint16_t.
 *
 * Usage: decode <input.ivf> <output.raw> */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "splot.h"

static int write_picture(FILE *out, const SplotPicture *picture) {
    int bytes = picture->bit_depth > 8 ? 2 : 1;
    int sub_x = picture->layout == SPLOT_PIXEL_LAYOUT_I420 || picture->layout == SPLOT_PIXEL_LAYOUT_I422;
    int sub_y = picture->layout == SPLOT_PIXEL_LAYOUT_I420;
    for (int plane = 0; plane < 3; plane++) {
        if (!picture->data[plane])
            continue;
        int shift_x = plane ? sub_x : 0, shift_y = plane ? sub_y : 0;
        size_t width = (picture->width + shift_x) >> shift_x;
        size_t height = (picture->height + shift_y) >> shift_y;
        const unsigned char *row = (const unsigned char *)picture->data[plane] +
                                   (ptrdiff_t)(picture->crop_top >> shift_y) * picture->stride[plane] +
                                   (size_t)(picture->crop_left >> shift_x) * bytes;
        for (size_t y = 0; y < height; y++, row += picture->stride[plane])
            if (fwrite(row, bytes, width, out) != width)
                return -1;
    }
    return 0;
}

static int drain_pictures(SplotDecoder *decoder, FILE *out, int *pictures) {
    for (;;) {
        SplotPicture picture;
        int result = splot_get_picture(decoder, &picture);
        if (result == SPLOT_EAGAIN || result == SPLOT_EOF)
            return 0;
        if (result != SPLOT_OK)
            return result;
        int written = write_picture(out, &picture);
        splot_picture_unref(&picture);
        if (written)
            return written;
        (*pictures)++;
    }
}

int main(int argc, char **argv) {
    if (argc != 3) {
        fprintf(stderr, "usage: %s <input.ivf> <output.raw>\n", argv[0]);
        return 2;
    }
    FILE *in = fopen(argv[1], "rb");
    FILE *out = fopen(argv[2], "wb");
    if (!in || !out) {
        fprintf(stderr, "cannot open the input or output file\n");
        return 1;
    }
    unsigned char header[32];
    if (fread(header, 1, sizeof header, in) != sizeof header || memcmp(header, "DKIF", 4)) {
        fprintf(stderr, "not an IVF file\n");
        return 1;
    }
    fseek(in, header[6] | header[7] << 8, SEEK_SET);

    SplotSettings settings;
    splot_default_settings(&settings);
    SplotDecoder *decoder = NULL;
    int result = splot_open(&decoder, &settings);
    int pictures = 0;
    unsigned char record[12];
    while (result == SPLOT_OK && fread(record, 1, sizeof record, in) == sizeof record) {
        size_t size = record[0] | record[1] << 8 | record[2] << 16 | (size_t)record[3] << 24;
        unsigned char *data = malloc(size ? size : 1);
        if (!data || fread(data, 1, size, in) != size) {
            free(data);
            fprintf(stderr, "truncated IVF frame\n");
            return 1;
        }
        SplotDataProps props = {0};
        memcpy(&props.pts, record + 4, sizeof props.pts);
        while ((result = splot_send_data(decoder, data, size, &props)) == SPLOT_EAGAIN)
            result = drain_pictures(decoder, out, &pictures);
        free(data);
        if (result == SPLOT_OK)
            result = drain_pictures(decoder, out, &pictures);
    }
    if (result == SPLOT_OK) {
        splot_drain(decoder);
        result = drain_pictures(decoder, out, &pictures);
    }
    splot_close(&decoder);
    fclose(in);
    if (fclose(out) || result != SPLOT_OK) {
        fprintf(stderr, "decode failed: %d\n", result);
        return 1;
    }
    fprintf(stderr, "splot %s: %d picture(s)\n", splot_version(), pictures);
    return 0;
}
