// The C++ equivalent of collatz.mn.
#include <cstdint>
#include <cstdio>
#include <cstdlib>

namespace {

std::uint64_t collatz(std::uint64_t limit) {
  std::uint64_t total = 0;
  std::uint64_t longest = 0;
  for (std::uint64_t start = 1; start < limit; start++) {
    std::uint64_t x = start;
    std::uint64_t steps = 0;
    while (x != 1) {
      if (x % 2 == 0) {
        x = x / 2;
      } else {
        x = 3 * x + 1;
      }
      steps += 1;
    }
    total += steps;
    if (steps > longest) {
      longest = steps;
    }
  }
  return total * 1000 + longest;
}

}  // namespace

int main(int argc, char** argv) {
  if (argc != 2) {
    std::fprintf(stderr, "usage: collatz <limit>\n");
    return 2;
  }
  auto limit = static_cast<std::uint64_t>(std::strtoull(argv[1], nullptr, 10));
  std::printf("%llu\n", static_cast<unsigned long long>(collatz(limit)));
  return 0;
}
