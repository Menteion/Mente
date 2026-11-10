// The C++ equivalent of sieve.mn.
#include <array>
#include <cstdint>
#include <cstdio>
#include <cstdlib>

namespace {

std::uint64_t sieve(std::uint32_t reps) {
  std::array<bool, 262144> composite{};
  std::uint64_t check = 0;
  for (std::uint32_t r = 0; r < reps; r++) {
    std::size_t limit = 262144 - r % 8u;
    for (std::size_t c = 0; c < limit; c++) {
      composite[c] = false;
    }
    for (std::size_t p = 2; p * p < limit; p++) {
      if (!composite[p]) {
        for (std::size_t m = p * p; m < limit; m += p) {
          composite[m] = true;
        }
      }
    }
    std::uint64_t count = 0;
    for (std::size_t n = 2; n < limit; n++) {
      if (!composite[n]) {
        count += 1;
      }
    }
    check = (check * 31u + count) % 1000000007u;
  }
  return check;
}

}  // namespace

int main(int argc, char** argv) {
  if (argc != 2) {
    std::fprintf(stderr, "usage: sieve <reps>\n");
    return 2;
  }
  auto reps = static_cast<std::uint32_t>(std::strtoul(argv[1], nullptr, 10));
  std::printf("%llu\n", static_cast<unsigned long long>(sieve(reps)));
  return 0;
}
