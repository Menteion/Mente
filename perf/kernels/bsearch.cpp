// The C++ equivalent of bsearch.mn.
#include <array>
#include <cstdint>
#include <cstdio>
#include <cstdlib>

namespace {

std::uint64_t bsearch_kernel(std::uint32_t reps) {
  std::array<std::uint32_t, 4096> a{};
  std::array<std::uint32_t, 4096> keys{};
  std::uint64_t seed = 12345;
  std::uint32_t value = 0;
  for (std::size_t i = 0; i < a.size(); i++) {
    seed = (seed * 1103515245u + 12345u) % 2147483648u;
    value += static_cast<std::uint32_t>(1 + seed % 8u);
    a[i] = value;
  }
  for (std::size_t i = 0; i < keys.size(); i++) {
    seed = (seed * 1103515245u + 12345u) % 2147483648u;
    keys[i] = static_cast<std::uint32_t>(seed % 20480u);
  }
  std::uint64_t check = 0;
  for (std::uint32_t r = 0; r < reps; r++) {
    std::uint64_t total = 0;
    for (std::size_t q = 0; q < keys.size(); q++) {
      std::uint32_t key = keys[q];
      std::size_t lo = 0;
      std::size_t hi = a.size();
      while (lo < hi) {
        std::size_t mid = (lo + hi) / 2;
        if (a[mid] < key) {
          lo = mid + 1;
        } else {
          hi = mid;
        }
      }
      total += lo;
      if (lo < a.size() && a[lo] == key) {
        total += 1000000;
      }
    }
    check = (check * 31u + total) % 1000000007u;
    keys[r % 4096u] = static_cast<std::uint32_t>(check % 20480u);
  }
  return check;
}

}  // namespace

int main(int argc, char** argv) {
  if (argc != 2) {
    std::fprintf(stderr, "usage: bsearch <reps>\n");
    return 2;
  }
  auto reps = static_cast<std::uint32_t>(std::strtoul(argv[1], nullptr, 10));
  std::printf("%llu\n", static_cast<unsigned long long>(bsearch_kernel(reps)));
  return 0;
}
