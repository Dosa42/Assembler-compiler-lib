/* SPDX-License-Identifier: MIT */
#define _GNU_SOURCE
#include "brokkr_usb_client.h"
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <sys/un.h>
#include <unistd.h>

static int request(const char *path, const char *command, int wants_fd,
                   char *json, size_t capacity) {
    if (!json || capacity < 2 || !path) { errno = EINVAL; return -1; }
    json[0] = '\0';
    struct sockaddr_un address = {.sun_family = AF_UNIX};
    if (strlen(path) >= sizeof(address.sun_path)) { errno = ENAMETOOLONG; return -1; }
    memcpy(address.sun_path, path, strlen(path) + 1);
    int sock = socket(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0);
    if (sock < 0) return -1;
    struct timeval timeout = {.tv_sec = 3};
    int result = -1, received_fd = -1, error = 0, count = 0;
    if (setsockopt(sock, SOL_SOCKET, SO_RCVTIMEO, &timeout, sizeof(timeout)) < 0 ||
        setsockopt(sock, SOL_SOCKET, SO_SNDTIMEO, &timeout, sizeof(timeout)) < 0 ||
        connect(sock, (struct sockaddr *)&address, sizeof(address)) < 0) goto done;
    struct ucred peer;
    socklen_t peer_size = sizeof(peer);
    if (getsockopt(sock, SOL_SOCKET, SO_PEERCRED, &peer, &peer_size) < 0) goto done;
    if (peer.uid != geteuid()) { errno = EACCES; goto done; }
    size_t command_size = strlen(command);
    if (send(sock, command, command_size, MSG_NOSIGNAL) != (ssize_t)command_size) goto done;
    char control[CMSG_SPACE(sizeof(int) * 8)] = {0};
    struct iovec vector = {.iov_base = json, .iov_len = capacity - 1};
    struct msghdr message = {.msg_iov = &vector, .msg_iovlen = 1,
                            .msg_control = control, .msg_controllen = sizeof(control)};
    ssize_t size = recvmsg(sock, &message, MSG_CMSG_CLOEXEC);
    if (size <= 0) { if (!size) errno = EPIPE; goto done; }
    json[size] = '\0';
    for (struct cmsghdr *header = CMSG_FIRSTHDR(&message); header;
         header = CMSG_NXTHDR(&message, header)) {
        if (header->cmsg_level != SOL_SOCKET || header->cmsg_type != SCM_RIGHTS) continue;
        size_t descriptor_count = (header->cmsg_len - CMSG_LEN(0)) / sizeof(int);
        int *descriptors = (int *)CMSG_DATA(header);
        for (size_t i = 0; i < descriptor_count; i++) {
            count++;
            if (received_fd < 0) received_fd = descriptors[i]; else close(descriptors[i]);
        }
    }
    if ((message.msg_flags & (MSG_CTRUNC | MSG_TRUNC)) || count != wants_fd) {
        errno = EMSGSIZE; goto done;
    }
    if (strstr(json, "\"error\":")) { errno = EACCES; goto done; }
    if (wants_fd && fcntl(received_fd, F_SETFD, FD_CLOEXEC) < 0) goto done;
    result = wants_fd ? received_fd : 0;
    received_fd = -1;
done:
    error = errno;
    if (received_fd >= 0) close(received_fd);
    close(sock);
    errno = error;
    return result;
}

int brokkr_usb_acquire(const char *path, char *json, size_t capacity) {
    return request(path, "GET\n", 1, json, capacity);
}

int brokkr_usb_status(const char *path, char *json, size_t capacity) {
    return request(path, "STATUS\n", 0, json, capacity);
}

int brokkr_usb_stop(const char *path, const char *session, char *json, size_t capacity) {
    if (!session || strlen(session) != 32 || strspn(session, "0123456789abcdef") != 32) {
        if (json && capacity) json[0] = '\0';
        errno = EINVAL; return -1;
    }
    char command[39];
    snprintf(command, sizeof(command), "STOP %s\n", session);
    return request(path, command, 0, json, capacity);
}
