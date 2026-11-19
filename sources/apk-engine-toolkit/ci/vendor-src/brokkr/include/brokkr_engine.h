#ifndef BROKKR_ENGINE_H
#define BROKKR_ENGINE_H

#ifdef __cplusplus
extern "C" {
#endif

/* argv omits the executable. Free the allocated JSON with brokkr_free.
 * Calls are serialized. Inherited USB descriptors remain caller-owned.
 * Returns 0 on success, 1 on operation failure, 2 on usage, 70 on allocation failure. */
int brokkr_execute(int argc, const char *const *argv, char **output_json);
void brokkr_free(char *output_json);

#ifdef __cplusplus
}
#endif
#endif
