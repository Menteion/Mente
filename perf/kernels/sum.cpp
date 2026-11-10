// The C++ equivalent of sum.mn.
#include <array>
#include <cstdint>
#include <cstdio>
#include <cstdlib>

namespace {

std::uint64_t sum(std::uint32_t reps) {
  std::array<std::uint32_t, 4096> a{};
  std::uint64_t seed = 12345;
  for (std::size_t i = 0; i < a.size(); i++) {
    seed = (seed * 1103515245u + 12345u) % 2147483648u;
    a[i] = static_cast<std::uint32_t>(seed % 1048576u);
  }
  std::uint64_t check = 0;
  for (std::uint32_t r = 0; r < reps; r++) {
    std::uint64_t total = 0;
    for (std::size_t j = 0; j < a.size(); j++) {
      total += a[j];
    }
    check = (check * 31u + total) % 1000000007u;
    a[r % 4096u] = static_cast<std::uint32_t>(check % 1048576u);
  }
  return check;
}

}  // namespace

int main(int argc, char** argv) {
  if (argc != 2) {
    std::fprintf(stderr, "usage: sum <reps>\n");
    return 2;
  }
  auto reps = static_cast<std::uint32_t>(std::strtoul(argv[1], nullptr, 10));
  std::printf("%llu\n", static_cast<unsigned long long>(sum(reps)));
  return 0;
}
