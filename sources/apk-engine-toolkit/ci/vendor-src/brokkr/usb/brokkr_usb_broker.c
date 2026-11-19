/* SPDX-License-Identifier: MIT */
#define _GNU_SOURCE
#include "brokkr_usb_client.h"
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <limits.h>
#include <signal.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/file.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <sys/un.h>
#include <time.h>
#include <unistd.h>

static volatile sig_atomic_t interrupted;
static int device_fd = -1;
static char session[33], device_path[PATH_MAX], socket_path[108];
static struct stat device_identity;
static time_t opened_at;
static int vendor_id = -1, product_id = -1;

static void on_signal(int signal_number) { interrupted = signal_number; }

static void json_string(char *destination, size_t capacity, const char *source) {
    size_t out = 0;
    for (const unsigned char *in = (const unsigned char *)source; *in && out + 7 < capacity; in++) {
        if (*in == '"' || *in == '\\') { destination[out++] = '\\'; destination[out++] = (char)*in; }
        else if (*in < 32) out += (size_t)snprintf(destination + out, capacity - out, "\\u%04x", *in);
        else destination[out++] = (char)*in;
    }
    destination[out] = '\0';
}

static void metadata(char *json, size_t capacity, const char *state, const char *error) {
    char escaped_path[PATH_MAX * 2], escaped_socket[256];
    char vendor[16] = "null", product[16] = "null", error_field[128] = "";
    json_string(escaped_path, sizeof(escaped_path), device_path);
    json_string(escaped_socket, sizeof(escaped_socket), socket_path);
    if (vendor_id >= 0) snprintf(vendor, sizeof(vendor), "%d", vendor_id);
    if (product_id >= 0) snprintf(product, sizeof(product), "%d", product_id);
    if (error) snprintf(error_field, sizeof(error_field), ",\"error\":\"%s\"", error);
    snprintf(json, capacity,
             "{\"schema\":\"brokkr.usb-broker/1\",\"state\":\"%s\",\"pid\":%ld,\"uid\":%ld,"
             "\"session\":\"%s\",\"socket\":\"%s\",\"device_path\":\"%s\","
             "\"identity\":{\"dev\":%ju,\"ino\":%ju,\"rdev\":%ju,\"mode\":%u},"
             "\"vendor_id\":%s,\"product_id\":%s,\"opened_at\":%jd,\"observed_at\":%jd%s}",
             state, (long)getpid(), (long)geteuid(), session, escaped_socket, escaped_path,
             (uintmax_t)device_identity.st_dev, (uintmax_t)device_identity.st_ino,
             (uintmax_t)device_identity.st_rdev, (unsigned)device_identity.st_mode,
             vendor, product, (intmax_t)opened_at, (intmax_t)time(NULL), error_field);
}

static int save_metadata(const char *json) {
    char path[PATH_MAX], temporary[PATH_MAX];
    snprintf(path, sizeof(path), "%s.json", socket_path);
    snprintf(temporary, sizeof(temporary), "%s.json.XXXXXX", socket_path);
    int fd = mkstemp(temporary);
    if (fd < 0) return -1;
    size_t length = strlen(json), written = 0;
    while (written < length) {
        ssize_t count = write(fd, json + written, length - written);
        if (count < 0 && errno == EINTR) continue;
        if (count <= 0) { close(fd); unlink(temporary); return -1; }
        written += (size_t)count;
    }
    int result = fsync(fd);
    if (close(fd) < 0) result = -1;
    if (!result) result = rename(temporary, path);
    if (result) unlink(temporary);
    return result;
}

static int read_number(const char *directory, const char *name, int base) {
    char path[PATH_MAX], buffer[32];
    snprintf(path, sizeof(path), "%s/%s", directory, name);
    int fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd < 0) return -1;
    ssize_t length = read(fd, buffer, sizeof(buffer) - 1);
    close(fd);
    if (length < 1) return -1;
    buffer[length] = '\0';
    char *end;
    long value = strtol(buffer, &end, base);
    if (end == buffer || value < 0 || value > 65535) return -1;
    return (int)value;
}

static void inspect_device(void) {
    char descriptor_path[64];
    snprintf(descriptor_path, sizeof(descriptor_path), "/proc/self/fd/%d", device_fd);
    ssize_t length = readlink(descriptor_path, device_path, sizeof(device_path) - 1);
    if (length >= 0) device_path[length] = '\0';
    unsigned bus, device;
    if (sscanf(device_path, "/dev/bus/usb/%u/%u", &bus, &device) != 2) return;
    DIR *directory = opendir("/sys/bus/usb/devices");
    if (!directory) return;
    struct dirent *entry;
    while ((entry = readdir(directory))) {
        if (entry->d_name[0] == '.') continue;
        char path[PATH_MAX];
        snprintf(path, sizeof(path), "/sys/bus/usb/devices/%s", entry->d_name);
        if (read_number(path, "busnum", 10) != (int)bus ||
            read_number(path, "devnum", 10) != (int)device) continue;
        vendor_id = read_number(path, "idVendor", 16);
        product_id = read_number(path, "idProduct", 16);
        break;
    }
    closedir(directory);
}

static int prepare_parent(void) {
    char parent[sizeof(socket_path)];
    memcpy(parent, socket_path, sizeof(parent));
    char *slash = strrchr(parent, '/');
    if (!slash || slash == parent) { errno = EINVAL; return -1; }
    *slash = '\0';
    if (mkdir(parent, 0700) < 0 && errno != EEXIST) return -1;
    struct stat info;
    if (lstat(parent, &info) < 0) return -1;
    if (!S_ISDIR(info.st_mode) || info.st_uid != geteuid() || (info.st_mode & 0077)) {
        errno = EACCES; return -1;
    }
    return 0;
}

static int send_response(int client, const char *json, int fd) {
    char control[CMSG_SPACE(sizeof(int))] = {0};
    struct iovec vector = {.iov_base = (void *)json, .iov_len = strlen(json)};
    struct msghdr message = {.msg_iov = &vector, .msg_iovlen = 1};
    if (fd >= 0) {
        message.msg_control = control;
        message.msg_controllen = sizeof(control);
        struct cmsghdr *header = CMSG_FIRSTHDR(&message);
        header->cmsg_level = SOL_SOCKET;
        header->cmsg_type = SCM_RIGHTS;
        header->cmsg_len = CMSG_LEN(sizeof(int));
        memcpy(CMSG_DATA(header), &fd, sizeof(fd));
    }
    return sendmsg(client, &message, MSG_NOSIGNAL) == (ssize_t)vector.iov_len ? 0 : -1;
}

static int serve(void) {
    int lock_fd = -1, server = -1, result = 1;
    bool bound = false, recorded = false;
    struct stat socket_identity = {0};
    char json[BROKKR_USB_METADATA_CAPACITY], lock_path[PATH_MAX];
    if (prepare_parent() < 0) { perror("socket directory"); goto done; }
    snprintf(lock_path, sizeof(lock_path), "%s.lock", socket_path);
    lock_fd = open(lock_path, O_CREAT | O_RDWR | O_CLOEXEC | O_NOFOLLOW, 0600);
    if (lock_fd < 0) { perror("socket lock"); goto done; }
    struct stat lock_identity;
    if (fstat(lock_fd, &lock_identity) < 0 || !S_ISREG(lock_identity.st_mode) ||
        lock_identity.st_uid != geteuid() || (lock_identity.st_mode & 0077)) {
        fputs("socket lock ownership rejected\n", stderr); goto done;
    }
    if (flock(lock_fd, LOCK_EX | LOCK_NB) < 0) { perror("broker already owns socket"); goto done; }
    struct sockaddr_un address = {.sun_family = AF_UNIX};
    memcpy(address.sun_path, socket_path, strlen(socket_path) + 1);
    struct stat existing;
    if (lstat(socket_path, &existing) == 0) {
        if (!S_ISSOCK(existing.st_mode) || existing.st_uid != geteuid()) {
            fputs("existing socket path ownership rejected\n", stderr); goto done;
        }
        int check = socket(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0);
        if (check < 0) goto done;
        int connected = connect(check, (struct sockaddr *)&address, sizeof(address));
        int connect_error = errno;
        close(check);
        if (!connected || connect_error != ECONNREFUSED) {
            fputs("existing socket is not safely stale\n", stderr); goto done;
        }
        if (unlink(socket_path) < 0) { perror("stale socket"); goto done; }
    } else if (errno != ENOENT) { perror("socket path"); goto done; }
    server = socket(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0);
    if (server < 0) { perror("socket"); goto done; }
    if (bind(server, (struct sockaddr *)&address, sizeof(address)) < 0) { perror("bind"); goto done; }
    bound = true;
    if (lstat(socket_path, &socket_identity) < 0 || chmod(socket_path, 0600) < 0 || listen(server, 8) < 0) {
        perror("listen"); goto done;
    }
    metadata(json, sizeof(json), "open", NULL);
    if (save_metadata(json) < 0) { perror("metadata"); goto done; }
    recorded = true;
    puts(json);
    fflush(stdout);
    result = 0;
    while (!interrupted) {
        int client = accept4(server, NULL, NULL, SOCK_CLOEXEC);
        if (client < 0) {
            if (errno == EINTR) continue;
            perror("accept"); result = 1; break;
        }
        struct timeval timeout = {.tv_sec = 3};
        (void)setsockopt(client, SOL_SOCKET, SO_RCVTIMEO, &timeout, sizeof(timeout));
        (void)setsockopt(client, SOL_SOCKET, SO_SNDTIMEO, &timeout, sizeof(timeout));
        struct ucred peer;
        socklen_t peer_size = sizeof(peer);
        if (getsockopt(client, SOL_SOCKET, SO_PEERCRED, &peer, &peer_size) < 0 || peer.uid != geteuid()) {
            close(client); continue;
        }
        char command[128];
        ssize_t size = recv(client, command, sizeof(command) - 1, MSG_TRUNC);
        if (size <= 0 || size >= (ssize_t)sizeof(command)) { close(client); continue; }
        command[size] = '\0';
        if (!strcmp(command, "GET\n")) {
            metadata(json, sizeof(json), "open", NULL);
            (void)send_response(client, json, device_fd);
        } else if (!strcmp(command, "STATUS\n")) {
            metadata(json, sizeof(json), "open", NULL);
            (void)send_response(client, json, -1);
        } else if (!strncmp(command, "STOP ", 5)) {
            char expected[39];
            snprintf(expected, sizeof(expected), "STOP %s\n", session);
            if (strcmp(command, expected)) {
                metadata(json, sizeof(json), "open", "session_mismatch");
                (void)send_response(client, json, -1);
            } else {
                close(device_fd);
                device_fd = -1;
                metadata(json, sizeof(json), "closed", NULL);
                (void)send_response(client, json, -1);
                close(client);
                break;
            }
        } else {
            metadata(json, sizeof(json), "open", "unknown_command");
            (void)send_response(client, json, -1);
        }
        close(client);
    }
done:
    if (device_fd >= 0) { close(device_fd); device_fd = -1; }
    if (recorded) {
        metadata(json, sizeof(json), "closed", NULL);
        if (save_metadata(json) < 0) { perror("closed metadata"); result = 1; }
    }
    if (server >= 0) close(server);
    if (bound) {
        struct stat current;
        if (!lstat(socket_path, &current) && current.st_dev == socket_identity.st_dev &&
            current.st_ino == socket_identity.st_ino) (void)unlink(socket_path);
    }
    if (lock_fd >= 0) close(lock_fd);
    return result;
}

static int parse_fd(const char *text) {
    if (!text || !*text) return -1;
    char *end;
    errno = 0;
    long value = strtol(text, &end, 10);
    if (errno || *end || value < 0 || value > INT_MAX) return -1;
    return (int)value;
}

static void print_unavailable(const char *path, int error) {
    char escaped_path[256], escaped_error[512];
    json_string(escaped_path, sizeof(escaped_path), path);
    json_string(escaped_error, sizeof(escaped_error), strerror(error));
    printf("{\"schema\":\"brokkr.usb-broker/1\",\"state\":\"unavailable\",\"socket\":\"%s\","
           "\"error_errno\":%d,\"error\":\"%s\"}\n", escaped_path, error, escaped_error);
}

int main(int argc, char **argv) {
    const char *path = getenv("BROKKR_USB_SOCKET");
    if (!path || !*path) path = BROKKR_USB_DEFAULT_SOCKET;
    const char *fd_text = getenv("TERMUX_USB_FD"), *stop_session = NULL;
    bool status = false;
    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "--socket") && i + 1 < argc) path = argv[++i];
        else if (!strcmp(argv[i], "--fd") && i + 1 < argc) fd_text = argv[++i];
        else if (!strcmp(argv[i], "--status")) status = true;
        else if (!strcmp(argv[i], "--stop") && i + 1 < argc) stop_session = argv[++i];
        else if (!strcmp(argv[i], "--help")) {
            puts("Usage: brokkr-usb-broker [--socket PATH] [--fd FD | FD]\n"
                 "       brokkr-usb-broker [--socket PATH] --status\n"
                 "       brokkr-usb-broker [--socket PATH] --stop SESSION\n"
                 "FD defaults to TERMUX_USB_FD; it must already be open in this process.");
            return 0;
        } else if (argv[i][0] != '-' && parse_fd(argv[i]) >= 0) fd_text = argv[i];
        else { fprintf(stderr, "Invalid argument: %s\n", argv[i]); return 2; }
    }
    if (strlen(path) >= sizeof(socket_path) || path[0] != '/') {
        fputs("Socket path must be absolute and shorter than 108 bytes\n", stderr); return 2;
    }
    memcpy(socket_path, path, strlen(path) + 1);
    if (status || stop_session) {
        char json[BROKKR_USB_METADATA_CAPACITY] = "";
        int result = stop_session ? brokkr_usb_stop(path, stop_session, json, sizeof(json)) :
                                    brokkr_usb_status(path, json, sizeof(json));
        if (json[0]) puts(json); else print_unavailable(path, errno);
        return result < 0 ? 1 : 0;
    }
    int inherited_fd = parse_fd(fd_text);
    if (inherited_fd < 0 || fstat(inherited_fd, &device_identity) < 0) {
        fputs("No valid inherited USB descriptor; launch with termux-usb -r -E -e BROKER DEVICE\n", stderr);
        return 2;
    }
    device_fd = fcntl(inherited_fd, F_DUPFD_CLOEXEC, 3);
    if (device_fd < 0) { perror("retain descriptor"); return 1; }
    close(inherited_fd);
    unsigned char random_bytes[16];
    int random_fd = open("/dev/urandom", O_RDONLY | O_CLOEXEC);
    if (random_fd < 0) { perror("session randomness"); close(device_fd); return 1; }
    size_t received = 0;
    while (received < sizeof(random_bytes)) {
        ssize_t count = read(random_fd, random_bytes + received, sizeof(random_bytes) - received);
        if (count < 0 && errno == EINTR) continue;
        if (count <= 0) { perror("session randomness"); close(random_fd); close(device_fd); return 1; }
        received += (size_t)count;
    }
    close(random_fd);
    for (size_t i = 0; i < sizeof(random_bytes); i++) snprintf(session + i * 2, 3, "%02x", random_bytes[i]);
    opened_at = time(NULL);
    inspect_device();
    umask(0077);
    struct sigaction action = {.sa_handler = on_signal};
    sigemptyset(&action.sa_mask);
    if (sigaction(SIGINT, &action, NULL) < 0 || sigaction(SIGTERM, &action, NULL) < 0) {
        perror("signal handler"); close(device_fd); return 1;
    }
    return serve();
}
