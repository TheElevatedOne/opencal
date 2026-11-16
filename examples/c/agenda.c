/* Print the next 7 days of the opencal calendar and add a task.
 *   cc agenda.c -o agenda $(pkg-config --cflags --libs opencal)
 *   ./agenda "Call the bank"
 */
#include <opencal.h>
#include <stdio.h>
#include <time.h>

int main(int argc, char **argv) {
    char *path = opencal_default_path(NULL);
    opencal_store *store = opencal_store_open(path);
    if (!store) {
        fprintf(stderr, "opencal: %s\n", opencal_last_error());
        return 1;
    }

    char from[11], to[11];
    time_t now = time(NULL), later = now + 7 * 24 * 3600;
    strftime(from, sizeof from, "%Y-%m-%d", localtime(&now));
    strftime(to, sizeof to, "%Y-%m-%d", localtime(&later));
    char *json = opencal_store_occurrences_json(store, from, to);
    printf("%s\n", json);
    opencal_string_free(json);

    if (argc > 1) {
        char *uid = opencal_store_add_task(store, NULL, argv[1], NULL);
        if (!uid || opencal_store_save(store, path) != 0) {
            fprintf(stderr, "opencal: %s\n", opencal_last_error());
        } else {
            printf("added task %s\n", uid);
        }
        opencal_string_free(uid);
    }

    opencal_store_free(store);
    opencal_string_free(path);
    return 0;
}
