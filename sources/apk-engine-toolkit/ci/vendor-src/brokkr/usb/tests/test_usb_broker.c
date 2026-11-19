/* SPDX-License-Identifier: MIT */
#define _GNU_SOURCE
#include "brokkr_usb_client.h"
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

static pid_t active_pid = -1;
static const char *broker;
static char directory[108], path[108], fixture[128];
static const char payload[] = "SCM_RIGHTS real descriptor readback\n";

static void fail(const char *condition, int line) {
    fprintf(stderr, "line %d: %s failed (errno %d: %s)\n", line, condition, errno, strerror(errno));
    if (active_pid > 0) { kill(active_pid, SIGKILL); waitpid(active_pid, NULL, 0); }
    exit(1);
}
#define CHECK(condition) do { if (!(condition)) fail(#condition, __LINE__); } while (0)

static pid_t launch(bool environment) {
    int source = open(fixture, O_RDWR);
    CHECK(source >= 0);
    pid_t pid = fork();
    CHECK(pid >= 0);
    if (!pid) {
        char fd_text[32];
        snprintf(fd_text, sizeof(fd_text), "%d", source);
        int quiet = open("/dev/null", O_WRONLY);
        if (quiet >= 0) { dup2(quiet, STDOUT_FILENO); dup2(quiet, STDERR_FILENO); close(quiet); }
        if (environment) {
            setenv("TERMUX_USB_FD", fd_text, 1);
            execl(broker, broker, "--socket", path, (char *)NULL);
        } else {
            unsetenv("TERMUX_USB_FD");
            execl(broker, broker, "--socket", path, fd_text, (char *)NULL);
        }
        _exit(127);
    }
    close(source);
    return pid;
}

static void await_open(char *json) {
    for (int attempt = 0; attempt < 200; attempt++) {
        if (!brokkr_usb_status(path, json, BROKKR_USB_METADATA_CAPACITY)) return;
        usleep(10000);
    }
    fail("broker opens within 2 seconds", __LINE__);
}

static void get_session(const char *json, char *session) {
    const char *start = strstr(json, "\"session\":\"");
    CHECK(start != NULL);
    memcpy(session, start + strlen("\"session\":\""), 32);
    session[32] = '\0';
    CHECK(strspn(session, "0123456789abcdef") == 32);
}

static void await_exit(pid_t pid, int expected) {
    int status;
    CHECK(waitpid(pid, &status, 0) == pid);
    CHECK(WIFEXITED(status) && WEXITSTATUS(status) == expected);
    if (pid == active_pid) active_pid = -1;
}

static void read_payload(int fd) {
    char buffer[sizeof(payload)];
    CHECK(pread(fd, buffer, sizeof(payload) - 1, 0) == (ssize_t)sizeof(payload) - 1);
    CHECK(!memcmp(buffer, payload, sizeof(payload) - 1));
}

int main(int argc, char **argv) {
    CHECK(argc == 2);
    broker = argv[1];
    const char *temporary = getenv("TMPDIR");
    if (temporary == NULL || temporary[0] == '\0') {
#ifdef __ANDROID__
        temporary = "/data/data/com.termux/files/usr/tmp";
#else
        temporary = "/tmp";
#endif
    }
    int length = snprintf(directory, sizeof(directory), "%s/brokkr-test-XXXXXX", temporary);
    CHECK(length > 0 && (size_t)length + sizeof("/socket") <= sizeof(path));
    CHECK(mkdtemp(directory) != NULL);
    snprintf(path, sizeof(path), "%s/socket", directory);
    snprintf(fixture, sizeof(fixture), "%s/fixture", directory);
    int initial = open(fixture, O_CREAT | O_EXCL | O_WRONLY, 0600);
    CHECK(initial >= 0);
    CHECK(write(initial, payload, sizeof(payload) - 1) == (ssize_t)sizeof(payload) - 1);
    CHECK(close(initial) == 0);
    char json[BROKKR_USB_METADATA_CAPACITY], session[33];
    CHECK(brokkr_usb_status(path, json, sizeof(json)) == -1 && errno == ENOENT);
    active_pid = launch(false);
    await_open(json);
    CHECK(strstr(json, "\"state\":\"open\"") != NULL);
    CHECK(strstr(json, fixture) != NULL);
    get_session(json, session);
    int first = brokkr_usb_acquire(path, json, sizeof(json));
    CHECK(first >= 0);
    CHECK(fcntl(first, F_GETFD) & FD_CLOEXEC);
    read_payload(first);
    int second = brokkr_usb_acquire(path, json, sizeof(json));
    CHECK(second >= 0 && second != first);
    struct stat first_info, second_info;
    CHECK(fstat(first, &first_info) == 0 && fstat(second, &second_info) == 0);
    CHECK(first_info.st_ino == second_info.st_ino && first_info.st_dev == second_info.st_dev);
    CHECK(lseek(first, 4, SEEK_SET) == 4);
    char bytes[7];
    CHECK(read(second, bytes, sizeof(bytes)) == (ssize_t)sizeof(bytes));
    CHECK(!memcmp(bytes, payload + 4, sizeof(bytes)));
    CHECK(lseek(first, 0, SEEK_CUR) == 4 + (off_t)sizeof(bytes));
    CHECK(close(first) == 0);
    int third = brokkr_usb_acquire(path, json, sizeof(json));
    CHECK(third >= 0);
    read_payload(third);
    CHECK(close(third) == 0);
    char tiny[8];
    CHECK(brokkr_usb_acquire(path, tiny, sizeof(tiny)) == -1 && errno == EMSGSIZE);
    char wrong[33];
    memset(wrong, '0', 32); wrong[32] = '\0';
    if (!strcmp(wrong, session)) wrong[0] = '1';
    CHECK(brokkr_usb_stop(path, wrong, json, sizeof(json)) == -1 && errno == EACCES);
    CHECK(strstr(json, "session_mismatch") != NULL);
    CHECK(!brokkr_usb_status(path, json, sizeof(json)));
    pid_t duplicate = launch(true);
    await_exit(duplicate, 1);
    char after_duplicate[33];
    CHECK(!brokkr_usb_status(path, json, sizeof(json)));
    get_session(json, after_duplicate);
    CHECK(!strcmp(after_duplicate, session));
    CHECK(!brokkr_usb_stop(path, session, json, sizeof(json)));
    CHECK(strstr(json, "\"state\":\"closed\"") != NULL);
    await_exit(active_pid, 0);
    CHECK(access(path, F_OK) < 0 && errno == ENOENT);
    read_payload(second);
    CHECK(close(second) == 0);
    CHECK(brokkr_usb_status(path, json, sizeof(json)) == -1 && errno == ENOENT);
    char metadata_path[128];
    snprintf(metadata_path, sizeof(metadata_path), "%s.json", path);
    FILE *record = fopen(metadata_path, "r");
    CHECK(record != NULL);
    CHECK(fgets(json, sizeof(json), record) != NULL);
    fclose(record);
    CHECK(strstr(json, "\"state\":\"closed\"") != NULL);
    puts("PASS real SCM_RIGHTS transfer, CLOEXEC, retained broker FD, shared open description, truncation rejection, ownership token, duplicate refusal, closed metadata");
    active_pid = launch(true);
    await_open(json);
    char replaced_session[33];
    get_session(json, replaced_session);
    CHECK(strcmp(replaced_session, session) != 0);
    CHECK(kill(active_pid, SIGKILL) == 0);
    int killed_status;
    CHECK(waitpid(active_pid, &killed_status, 0) == active_pid);
    active_pid = -1;
    CHECK(WIFSIGNALED(killed_status));
    CHECK(brokkr_usb_status(path, json, sizeof(json)) == -1 && errno == ECONNREFUSED);
    active_pid = launch(true);
    await_open(json);
    get_session(json, session);
    CHECK(strcmp(replaced_session, session) != 0);
    CHECK(brokkr_usb_stop(path, replaced_session, json, sizeof(json)) == -1);
    CHECK(kill(active_pid, SIGTERM) == 0);
    await_exit(active_pid, 0);
    CHECK(access(path, F_OK) < 0 && errno == ENOENT);
    puts("PASS TERMUX_USB_FD environment handoff, stale socket recovery, old-token refusal, SIGTERM cleanup");
    unlink(metadata_path);
    char lock_path[128];
    snprintf(lock_path, sizeof(lock_path), "%s.lock", path);
    unlink(lock_path);
    unlink(fixture);
    CHECK(rmdir(directory) == 0);
    return 0;
}
