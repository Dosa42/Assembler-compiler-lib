#include "brokkr_engine.h"
#include <cstdio>

int main(int argc, char **argv) {
  char *output = nullptr;
  const int code = brokkr_execute(argc - 1, argv + 1, &output);
  if (output) {
    std::puts(output);
    brokkr_free(output);
  } else {
    std::fputs("Unable to allocate result JSON\n", stderr);
  }
  return code;
}
