// The C++ equivalent of popcount.mn.
#include <array>
#include <cstdint>
#include <cstdio>
#include <cstdlib>

namespace {

std::uint64_t popcount(std::uint32_t reps) {
  std::array<std::uint64_t, 4096> a{};
  std::uint64_t seed = 12345;
  for (std::size_t i = 0; i < a.size(); i++) {
    seed = (seed * 1103515245u + 12345u) % 2147483648u;
    std::uint64_t high = seed;
    seed = (seed * 1103515245u + 12345u) % 2147483648u;
    std::uint64_t low = seed;
    seed = (seed * 1103515245u + 12345u) % 2147483648u;
    a[i] = high * 8589934592u + low * 4u + seed % 4u;
  }
  std::uint64_t check = 0;
  for (std::uint32_t r = 0; r < reps; r++) {
    std::uint64_t total = 0;
    for (std::size_t j = 0; j < a.size(); j++) {
      std::uint64_t x = a[j];
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

}  // namespace

int main(int argc, char** argv) {
  if (argc != 2) {
    std::fprintf(stderr, "usage: popcount <reps>\n");
    return 2;
  }
  auto reps = static_cast<std::uint32_t>(std::strtoul(argv[1], nullptr, 10));
  std::printf("%llu\n", static_cast<unsigned long long>(popcount(reps)));
  return 0;
}
