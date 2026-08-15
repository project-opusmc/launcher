#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static int java_argument_index(int argc, char *const argv[]) {
    int index = 1;
    while (index < argc && strncmp(argv[index], "-psn_", 5) == 0) {
        index++;
    }
    return index;
}

int main(int argc, char *argv[]) {
    int java_index = java_argument_index(argc, argv);
    if (java_index >= argc) {
        fputs("[OPUS/STUB] Java executable argument is missing\n", stderr);
        return 64;
    }

    const char *working_directory = getenv("OPUS_GAME_WORKDIR");
    if (working_directory != NULL && working_directory[0] != '\0'
        && chdir(working_directory) != 0) {
        fprintf(
            stderr,
            "[OPUS/STUB] could not use game directory %s: %s\n",
            working_directory,
            strerror(errno));
        return 73;
    }

    execv(argv[java_index], &argv[java_index]);
    fprintf(stderr, "[OPUS/STUB] could not start Java: %s\n", strerror(errno));
    return 127;
}
