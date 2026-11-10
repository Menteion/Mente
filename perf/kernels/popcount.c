/* The C equivalent of popcount.mn. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

static uint64_t popcount(uint32_t reps) {
  uint64_t a[4096] = {0};
  uint64_t seed = 12345;
  for (size_t i = 0; i < 4096; i++) {
    seed = (seed * 1103515245u + 12345u) % 2147483648u;
    uint64_t high = seed;
    seed = (seed * 1103515245u + 12345u) % 2147483648u;
    uint64_t low = seed;
    seed = (seed * 1103515245u + 12345u) % 2147483648u;
    a[i] = high * 8589934592u + low * 4u + seed % 4u;
  }
  uint64_t check = 0;
  for (uint32_t r = 0; r < reps; r++) {
    uint64_t total = 0;
    for (size_t j = 0; j < 4096; j++) {
      uint64_t x = a[j];
      x = x - ((x >> 1) & 0x5555555555555555u);
      x = (x & 0x3333333333333333u) + ((x >> 2) & 0x3333333333333333u);
      x = (x + (x >> 4)) & 0x0F0F0F0F0F0F0F0Fu;
      x = x + (x >> 8);
      x = x + (x >> 16);
      x = x + (x >> 32);
      total += x & 0x7Fu;
    }
    check = (check * 31u + total) % 1000000007u;
    a[r % 4096u] = check * 8589934592u + total;
  }
  return check;
}

int main(int argc, char **argv) {
  if (argc != 2) {
    fprintf(stderr, "usage: popcount <reps>\n");
    return 2;
  }
  printf("%llu\n", (unsigned long long)popcount((uint32_t)strtoul(argv[1], NULL, 10)));
  return 0;
}
