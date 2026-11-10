// The C++ equivalent of matmul.mn.
#include <array>
#include <cstdint>
#include <cstdio>
#include <cstdlib>

namespace {

std::uint64_t matmul(std::uint32_t reps) {
  std::array<std::uint32_t, 1024> a{};
  std::array<std::uint32_t, 1024> b{};
  std::array<std::uint32_t, 1024> c{};
  std::uint64_t seed = 12345;
  for (std::size_t i = 0; i < a.size(); i++) {
    seed = (seed * 1103515245u + 12345u) % 2147483648u;
    a[i] = static_cast<std::uint32_t>(seed % 1024u);
    seed = (seed * 1103515245u + 12345u) % 2147483648u;
    b[i] = static_cast<std::uint32_t>(seed % 1024u);
  }
  std::uint64_t check = 0;
  for (std::uint32_t r = 0; r < reps; r++) {
    for (std::size_t row = 0; row < 32; row++) {
      for (std::size_t col = 0; col < 32; col++) {
        std::uint32_t acc = 0;
        for (std::size_t k = 0; k < 32; k++) {
          acc += a[row * 32 + k] * b[k * 32 + col];
        }
        c[row * 32 + col] = acc;
      }
    }
    std::uint64_t total = 0;
    for (std::size_t e = 0; e < c.size(); e++) {
      total += c[e];
    }
    check = (check * 31u + total) % 1000000007u;
    a[r % 1024u] = static_cast<std::uint32_t>(check % 1024u);
  }
  return check;
}

}  // namespace

int main(int argc, char** argv) {
  if (argc != 2) {
    std::fprintf(stderr, "usage: matmul <reps>\n");
    return 2;
  }
  auto reps = static_cast<std::uint32_t>(std::strtoul(argv[1], nullptr, 10));
  std::printf("%llu\n", static_cast<unsigned long long>(matmul(reps)));
  return 0;
}
