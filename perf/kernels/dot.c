/* The C equivalent of dot.mn. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

static uint64_t dot(uint32_t reps) {
  uint32_t a[4096] = {0};
  uint32_t b[4096] = {0};
  uint64_t seed = 12345;
  for (size_t i = 0; i < 4096; i++) {
    seed = (seed * 1103515245u + 12345u) % 2147483648u;
    a[i] = (uint32_t)(seed % 65536u);
    seed = (seed * 1103515245u + 12345u) % 2147483648u;
    b[i] = (uint32_t)(seed % 65536u);
  }
  uint64_t check = 0;
  for (uint32_t r = 0; r < reps; r++) {
    uint64_t total = 0;
    for (size_t j = 0; j < 4096; j++) {
      total += (uint64_t)a[j] * b[j];
    }
    check = (check * 31u + total) % 1000000007u;
    a[r % 4096u] = (uint32_t)(check % 65536u);
  }
  return check;
}

int main(int argc, char **argv) {
  if (argc != 2) {
    fprintf(stderr, "usage: dot <reps>\n");
    return 2;
  }
  printf("%llu\n", (unsigned long long)dot((uint32_t)strtoul(argv[1], NULL, 10)));
  return 0;
}
