// The C++ equivalent of isort.mn.
#include <array>
#include <cstdint>
#include <cstdio>
#include <cstdlib>

namespace {

std::uint64_t isort(std::uint32_t reps) {
  std::array<std::uint32_t, 256> source{};
  std::uint64_t seed = 12345;
  for (std::size_t i = 0; i < source.size(); i++) {
    seed = (seed * 1103515245u + 12345u) % 2147483648u;
    source[i] = static_cast<std::uint32_t>(seed);
  }
  std::uint64_t check = 0;
  for (std::uint32_t r = 0; r < reps; r++) {
    std::array<std::uint32_t, 256> w = source;
    for (std::size_t k = 1; k < w.size(); k++) {
      std::uint32_t item = w[k];
      std::size_t j = k;
      while (j > 0 && w[j - 1] > item) {
        w[j] = w[j - 1];
        j -= 1;
      }
      w[j] = item;
    }
    check = (check * 31u + w[r % 256u] + w[255]) % 1000000007u;
    source[r % 256u] = static_cast<std::uint32_t>(check);
  }
  return check;
}

}  // namespace

int main(int argc, char** argv) {
  if (argc != 2) {
    std::fprintf(stderr, "usage: isort <reps>\n");
    return 2;
  }
  auto reps = static_cast<std::uint32_t>(std::strtoul(argv[1], nullptr, 10));
  std::printf("%llu\n", static_cast<unsigned long long>(isort(reps)));
  return 0;
}
