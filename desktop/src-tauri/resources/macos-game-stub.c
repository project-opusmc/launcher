#include <errno.h>
#include <AppKit/AppKit.h>
#include <CoreGraphics/CoreGraphics.h>
#include <fcntl.h>
#include <signal.h>
#include <spawn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

extern char **environ;

static volatile sig_atomic_t child_pid = -1;

enum activation_result {
    ACTIVATION_OK = 0,
    ACTIVATION_TIMEOUT = 1,
    ACTIVATION_CHILD_EXITED = 2,
    ACTIVATION_CONSOLE_LOCKED = 3,
    ACTIVATION_WAIT_FAILED = 4,
};

static void write_marker(const char *path, const char *contents) {
    if (path == NULL || path[0] == '\0') {
        return;
    }
    int descriptor = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0600);
    if (descriptor < 0) {
        return;
    }
    size_t length = strlen(contents);
    (void)write(descriptor, contents, length);
    (void)fsync(descriptor);
    (void)close(descriptor);
}

static int macos_console_is_locked(void) {
    CFDictionaryRef session = CGSessionCopyCurrentDictionary();
    if (session == NULL) {
        return 0;
    }
    const void *value = CFDictionaryGetValue(session, CFSTR("CGSSessionScreenIsLocked"));
    int locked = value != NULL
        && CFGetTypeID(value) == CFBooleanGetTypeID()
        && CFBooleanGetValue((CFBooleanRef)value);
    CFRelease(session);
    return locked;
}

static void forward_signal(int signal_number) {
    pid_t pid = (pid_t)child_pid;
    if (pid > 1) {
        (void)kill(pid, signal_number);
    }
}

static int activate_java_window(pid_t pid, int *child_status, int *child_reaped) {
    // Forge + OptiFine can create the Java window after normal JVM and shader
    // initialization completes. Keep retrying long enough to cover that cold start rather
    // than silently leaving a healthy game behind the previously focused app.
    const CFTimeInterval deadline = CFAbsoluteTimeGetCurrent() + 60.0;
    if (child_reaped != NULL) {
        *child_reaped = 0;
    }
    int activation_requested = 0;
    while (CFAbsoluteTimeGetCurrent() < deadline) {
        if (macos_console_is_locked()) {
            return ACTIVATION_CONSOLE_LOCKED;
        }

        // kill(pid, 0) reports a crashed child as alive until its parent
        // reaps the zombie. Poll waitpid so a native JVM crash is detected
        // immediately instead of waiting out the full activation deadline.
        int status = 0;
        pid_t waited = waitpid(pid, &status, WNOHANG);
        if (waited == pid) {
            if (child_status != NULL) {
                *child_status = status;
            }
            if (child_reaped != NULL) {
                *child_reaped = 1;
            }
            return ACTIVATION_CHILD_EXITED;
        }
        if (waited < 0 && errno != EINTR) {
            fprintf(
                stderr,
                "[OPUS/STUB] could not inspect Java child: %s\n",
                strerror(errno));
            return ACTIVATION_WAIT_FAILED;
        }

        // Old Java 8/LWJGL can crash in MacOSXContextImplementation.setView if
        // another process repeatedly activates it while AppKit is still
        // constructing the first OpenGL window. Detect an owned window first,
        // activate exactly once, then require that window to become onscreen.
        CGWindowListOption window_options = activation_requested
            ? kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements
            : kCGWindowListOptionAll | kCGWindowListExcludeDesktopElements;
        CFArrayRef windows = CGWindowListCopyWindowInfo(
            window_options,
            kCGNullWindowID);
        int found = 0;
        if (windows != NULL) {
            CFIndex count = CFArrayGetCount(windows);
            for (CFIndex index = 0; index < count; index++) {
                CFDictionaryRef window = CFArrayGetValueAtIndex(windows, index);
                CFNumberRef owner_pid_number =
                    (CFNumberRef)CFDictionaryGetValue(window, kCGWindowOwnerPID);
                CFNumberRef layer_number =
                    (CFNumberRef)CFDictionaryGetValue(window, kCGWindowLayer);
                CFDictionaryRef bounds_dictionary = (CFDictionaryRef)CFDictionaryGetValue(
                    window,
                    kCGWindowBounds);
                int owner_pid = 0;
                int32_t layer = 0;
                CGRect bounds = CGRectZero;
                if (owner_pid_number == NULL
                    || !CFNumberGetValue(owner_pid_number, kCFNumberIntType, &owner_pid)
                    || owner_pid != pid
                    || layer_number == NULL
                    || !CFNumberGetValue(layer_number, kCFNumberSInt32Type, &layer)
                    || layer != 0
                    || bounds_dictionary == NULL
                    || !CGRectMakeWithDictionaryRepresentation(bounds_dictionary, &bounds)
                    || bounds.size.width < 10.0
                    || bounds.size.height < 10.0) {
                    continue;
                }

                found = 1;
                break;
            }
            CFRelease(windows);
        }
        if (found) {
            if (activation_requested) {
                return ACTIVATION_OK;
            }
            NSRunningApplication *application =
                [NSRunningApplication runningApplicationWithProcessIdentifier:pid];
            if (application != nil) {
                [application activateWithOptions:
                    NSApplicationActivateAllWindows | NSApplicationActivateIgnoringOtherApps];
                activation_requested = 1;
            }
        }
        usleep(50 * 1000);
    }
    return ACTIVATION_TIMEOUT;
}

static int terminate_child(pid_t pid, int *child_status) {
    if (pid <= 1) {
        return 0;
    }
    (void)kill(pid, SIGTERM);
    const CFTimeInterval deadline = CFAbsoluteTimeGetCurrent() + 5.0;
    while (CFAbsoluteTimeGetCurrent() < deadline) {
        int status = 0;
        pid_t result = waitpid(pid, &status, WNOHANG);
        if (result == pid) {
            if (child_status != NULL) {
                *child_status = status;
            }
            return 1;
        }
        if (result < 0 && errno == ECHILD) {
            // The child was already reaped by another wait path. Treat it as
            // terminal so the caller does not issue a second blocking wait.
            return 1;
        }
        if (result < 0 && errno != EINTR) {
            break;
        }
        usleep(50 * 1000);
    }
    (void)kill(pid, SIGKILL);
    return 0;
}

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

    // Do not use fork() after LaunchServices/AppKit has initialized this
    // process. Java's -XstartOnFirstThread path must initialize Cocoa cleanly,
    // while forked children inherit AppKit/CoreFoundation state that can crash
    // during LWJGL's first display-mode query. posix_spawn starts a fresh Java
    // process without copying that state and keeps this foreground stub alive
    // to activate and supervise the child's real game window.
    pid_t pid = -1;
    int spawn_result = posix_spawn(
        &pid,
        argv[java_index],
        NULL,
        NULL,
        &argv[java_index],
        environ);
    if (spawn_result != 0) {
        fprintf(stderr, "[OPUS/STUB] could not start Java: %s\n", strerror(spawn_result));
        return 127;
    }

    child_pid = (sig_atomic_t)pid;
    (void)signal(SIGTERM, forward_signal);
    (void)signal(SIGINT, forward_signal);
    (void)signal(SIGHUP, forward_signal);

    int child_status = 0;
    int child_reaped = 0;
    int activation_result = activate_java_window(pid, &child_status, &child_reaped);
    if (activation_result != 0) {
        const char *failure_marker = getenv("OPUS_GAME_WINDOW_FAILURE_FILE");
        write_marker(
            failure_marker,
            activation_result == ACTIVATION_CHILD_EXITED
                ? "java exited before its window became visible\n"
                : activation_result == ACTIVATION_CONSOLE_LOCKED
                    ? "macOS console is locked; unlock before launching Minecraft\n"
                    : activation_result == ACTIVATION_WAIT_FAILED
                        ? "could not inspect Java process while waiting for its window\n"
                        : "java window did not become visible before the activation deadline\n");
        fprintf(
            stderr,
            "[OPUS/STUB] game window activation failed (%d); stopping Java\n",
            activation_result);
        if (activation_result == ACTIVATION_CHILD_EXITED) {
            fprintf(
                stderr,
                "[OPUS/STUB] Java exited before its window became visible (status=%d)\n",
                child_status);
        } else if (!child_reaped) {
            child_reaped = terminate_child(pid, &child_status);
        }
    } else {
        const char *ready_marker = getenv("OPUS_GAME_WINDOW_READY_FILE");
        write_marker(ready_marker, "ready\n");
    }

    int status = child_status;
    pid_t waited = child_reaped ? pid : -1;
    if (!child_reaped) {
        do {
            waited = waitpid(pid, &status, 0);
        } while (waited < 0 && errno == EINTR);
    }
    child_pid = -1;
    if (waited < 0) {
        fprintf(stderr, "[OPUS/STUB] could not wait for Java: %s\n", strerror(errno));
        return 127;
    }
    if (activation_result != 0) {
        return 78;
    }
    if (WIFEXITED(status)) {
        return WEXITSTATUS(status);
    }
    if (WIFSIGNALED(status)) {
        return 128 + WTERMSIG(status);
    }
    return 127;
}
