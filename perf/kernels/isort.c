/* The C equivalent of isort.mn. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static uint64_t isort(uint32_t reps) {
  uint32_t source[256] = {0};
  uint64_t seed = 12345;
  for (size_t i = 0; i < 256; i++) {
    seed = (seed * 1103515245u + 12345u) % 2147483648u;
    source[i] = (uint32_t)seed;
  }
  uint64_t check = 0;
  for (uint32_t r = 0; r < reps; r++) {
    uint32_t w[256];
    memcpy(w, source, sizeof w);
    for (size_t k = 1; k < 256; k++) {
      uint32_t item = w[k];
      size_t j = k;
      while (j > 0 && w[j - 1] > item) {
        w[j] = w[j - 1];
        j -= 1;
      }
      w[j] = item;
    }
    check = (check * 31u + w[r % 256u] + w[255]) % 1000000007u;
    source[r % 256u] = (uint32_t)check;
  }
  return check;
}

int main(int argc, char **argv) {
  if (argc != 2) {
    fprintf(stderr, "usage: isort <reps>\n");
    return 2;
  }
  printf("%llu\n", (unsigned long long)isort((uint32_t)strtoul(argv[1], NULL, 10)));
  return 0;
}
