/* The C equivalent of matmul.mn. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

static uint64_t matmul(uint32_t reps) {
  uint32_t a[1024] = {0};
  uint32_t b[1024] = {0};
  uint32_t c[1024] = {0};
  uint64_t seed = 12345;
  for (size_t i = 0; i < 1024; i++) {
    seed = (seed * 1103515245u + 12345u) % 2147483648u;
    a[i] = (uint32_t)(seed % 1024u);
    seed = (seed * 1103515245u + 12345u) % 2147483648u;
    b[i] = (uint32_t)(seed % 1024u);
  }
  uint64_t check = 0;
  for (uint32_t r = 0; r < reps; r++) {
    for (size_t row = 0; row < 32; row++) {
      for (size_t col = 0; col < 32; col++) {
        uint32_t acc = 0;
        for (size_t k = 0; k < 32; k++) {
          acc += a[row * 32 + k] * b[k * 32 + col];
        }
        c[row * 32 + col] = acc;
      }
    }
    uint64_t total = 0;
    for (size_t e = 0; e < 1024; e++) {
      total += c[e];
    }
    check = (check * 31u + total) % 1000000007u;
    a[r % 1024u] = (uint32_t)(check % 1024u);
  }
  return check;
}

int main(int argc, char **argv) {
  if (argc != 2) {
    fprintf(stderr, "usage: matmul <reps>\n");
    return 2;
  }
  printf("%llu\n", (unsigned long long)matmul((uint32_t)strtoul(argv[1], NULL, 10)));
  return 0;
}
