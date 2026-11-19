#include <stdio.h>
#include "brokkr_engine.h"

int main(int argc, char **argv) {
    char *output = NULL;
    int status = brokkr_execute(argc - 1, (const char *const *)(argv + 1), &output);
    if (output != NULL) {
        puts(output);
        brokkr_free(output);
    }
    return status;
}
