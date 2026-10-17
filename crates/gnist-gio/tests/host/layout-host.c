/* A GTK host for the gnist-gio layout tests.
 *
 * Built against GTK 3 or GTK 4. With LAYOUT_HOST_PATCHED=1 it gives GtkSettings
 * a "gtk-herda-layout" string property, standing in for the patched toolkit;
 * otherwise GtkSettings is upstream's. LAYOUT_HOST_EARLY_READ=1 also stands in
 * for the toolkit's own read of gtk-layout.ini at startup.
 *
 * It loads gnist-gio the way applications do, through GIO_EXTRA_MODULES:
 * LAYOUT_HOST_MODULES=early (before gtk_init), thread (before gtk_init, from
 * a worker thread) or late (after it). It reports on stdout:
 *
 *   NOTIFY <layout>          GtkSettings:gtk-herda-layout changed
 *   WINDOW <n> <layout>      a window built now would read this layout
 *
 *   OPACITY <o>...           the first window's direct children's opacities
 *
 * Commands on stdin: "window" builds a window, "opacity" reports,
 * "reduce-motion" turns animations off, "quit" (or EOF) exits.
 */
#include <gtk/gtk.h>
#include <glib/gstdio.h>
#include <sys/stat.h>
#include <stdio.h>
#include <string.h>

#define LAYOUT_PROPERTY "gtk-herda-layout"

static GMainLoop *loop;
static int windows;
static char *initial_layout;
static GtkWidget *first_window;

/* The toolkit's rules: anything but a complete version 1 group is native. */
static char *
read_layout_file (void)
{
  char *path = g_build_filename (g_get_user_config_dir (), "herda", "gtk-layout.ini", NULL);
  GKeyFile *file = g_key_file_new ();
  char *schema = NULL, *layout = NULL;
  gboolean desktop = FALSE;

  GStatBuf st;

  /* Only a regular file of at most 64 KiB, as the toolkit reads it. */
  if (g_stat (path, &st) == 0 && S_ISREG (st.st_mode) && st.st_size <= 64 * 1024 &&
      g_key_file_load_from_file (file, path, G_KEY_FILE_NONE, NULL))
    {
      schema = g_key_file_get_string (file, "GtkLayout", "Schema", NULL);
      layout = g_key_file_get_string (file, "GtkLayout", "Layout", NULL);
      desktop = g_strcmp0 (schema, "org.herda.GtkLayout") == 0
                && g_key_file_get_integer (file, "GtkLayout", "Version", NULL) == 1
                && g_strcmp0 (layout, "herda-desktop") == 0;
    }
  g_free (schema);
  g_free (layout);
  g_key_file_free (file);
  g_free (path);
  return g_strdup (desktop ? "herda-desktop" : "native");
}

static char *
current_layout (void)
{
  GtkSettings *settings = gtk_settings_get_default ();
  char *layout = NULL;

  if (!g_object_class_find_property (G_OBJECT_GET_CLASS (settings), LAYOUT_PROPERTY))
    return g_strdup ("unsupported");
  g_object_get (settings, LAYOUT_PROPERTY, &layout, NULL);
  return layout;
}

static void
on_layout (GObject *settings, GParamSpec *pspec, gpointer data)
{
  char *layout = current_layout ();
  printf ("NOTIFY %s\n", layout);
  fflush (stdout);
  g_free (layout);
}

static void
build_window (void)
{
  char *layout = current_layout ();
#if GTK_MAJOR_VERSION >= 4
  GtkWidget *window = gtk_window_new ();
  gtk_window_set_titlebar (GTK_WINDOW (window), gtk_header_bar_new ());
  gtk_window_set_child (GTK_WINDOW (window), gtk_label_new ("layout host"));
  gtk_window_present (GTK_WINDOW (window));
#else
  GtkWidget *window = gtk_window_new (GTK_WINDOW_TOPLEVEL);
  gtk_window_set_titlebar (GTK_WINDOW (window), gtk_header_bar_new ());
  gtk_container_add (GTK_CONTAINER (window), gtk_label_new ("layout host"));
  gtk_widget_show_all (window);
#endif
  if (!first_window)
    first_window = window;
  printf ("WINDOW %d %s\n", ++windows, layout);
  fflush (stdout);
  g_free (layout);
}

static void
print_opacity (GtkWidget *child, gpointer data)
{
  printf (" %.3f", gtk_widget_get_opacity (child));
}

static void
report_opacity (void)
{
  printf ("OPACITY");
#if GTK_MAJOR_VERSION >= 4
  for (GtkWidget *child = gtk_widget_get_first_child (first_window); child;
       child = gtk_widget_get_next_sibling (child))
    print_opacity (child, NULL);
#else
  gtk_container_forall (GTK_CONTAINER (first_window), print_opacity, NULL);
#endif
  printf ("\n");
  fflush (stdout);
}

static gboolean
on_command (GIOChannel *channel, GIOCondition condition, gpointer data)
{
  char *line = NULL;
  GIOStatus status = g_io_channel_read_line (channel, &line, NULL, NULL, NULL);

  if (status != G_IO_STATUS_NORMAL)
    {
      g_main_loop_quit (loop);
      return G_SOURCE_REMOVE;
    }
  g_strstrip (line);
  if (strcmp (line, "window") == 0)
    build_window ();
  else if (strcmp (line, "reduce-motion") == 0)
    g_object_set (gtk_settings_get_default (), "gtk-enable-animations", FALSE, NULL);
  else if (strcmp (line, "opacity") == 0)
    report_opacity ();
  else if (strcmp (line, "quit") == 0)
    g_main_loop_quit (loop);
  g_free (line);
  return G_SOURCE_CONTINUE;
}

#if GTK_MAJOR_VERSION >= 4
/* GTK 4 dropped gtk_settings_install_property(), so the stand-in property is
 * served beside GtkSettings' own, keeping its storage out of GTK's table. */
enum { PROP_LAYOUT = 0x7000 };
static GObjectSetPropertyFunc upstream_set;
static GObjectGetPropertyFunc upstream_get;
static char *stand_in_layout;

static void
stand_in_set (GObject *object, guint id, const GValue *value, GParamSpec *pspec)
{
  if (id != PROP_LAYOUT)
    {
      upstream_set (object, id, value, pspec);
      return;
    }
  g_free (stand_in_layout);
  stand_in_layout = g_value_dup_string (value);
}

static void
stand_in_get (GObject *object, guint id, GValue *value, GParamSpec *pspec)
{
  if (id != PROP_LAYOUT)
    {
      upstream_get (object, id, value, pspec);
      return;
    }
  g_value_set_string (value, stand_in_layout);
}

static void
patch_settings (void)
{
  GObjectClass *klass = g_type_class_ref (GTK_TYPE_SETTINGS);

  stand_in_layout = g_strdup (initial_layout);
  upstream_set = klass->set_property;
  upstream_get = klass->get_property;
  klass->set_property = stand_in_set;
  klass->get_property = stand_in_get;
  g_object_class_install_property (klass, PROP_LAYOUT,
                                   g_param_spec_string (LAYOUT_PROPERTY, NULL, NULL, initial_layout,
                                                        G_PARAM_READWRITE | G_PARAM_STATIC_STRINGS));
}
#else
static void
patch_settings (void)
{
  G_GNUC_BEGIN_IGNORE_DEPRECATIONS
  gtk_settings_install_property (g_param_spec_string (LAYOUT_PROPERTY, NULL, NULL, initial_layout,
                                                      G_PARAM_READWRITE | G_PARAM_STATIC_STRINGS));
  G_GNUC_END_IGNORE_DEPRECATIONS
}
#endif

int
main (int argc, char **argv)
{
  gboolean patched = g_strcmp0 (g_getenv ("LAYOUT_HOST_PATCHED"), "1") == 0;
  const char *modules = g_getenv ("LAYOUT_HOST_MODULES");
  GIOChannel *input;

  initial_layout = g_strcmp0 (g_getenv ("LAYOUT_HOST_EARLY_READ"), "1") == 0
                   ? read_layout_file () : g_strdup ("native");

  /* Applications load GIO modules before GTK is initialized (a
   * GApplication's registration), from a worker thread, or after it. */
  if (g_strcmp0 (modules, "early") == 0)
    g_vfs_get_default ();
  else if (g_strcmp0 (modules, "thread") == 0)
    g_thread_join (g_thread_new ("modules", (GThreadFunc) g_vfs_get_default, NULL));
  if (patched)
    patch_settings ();
#if GTK_MAJOR_VERSION >= 4
  gtk_init ();
#else
  gtk_init (&argc, &argv);
#endif
  if (g_strcmp0 (modules, "late") == 0)
    g_vfs_get_default ();

  g_signal_connect (gtk_settings_get_default (), "notify::" LAYOUT_PROPERTY,
                    G_CALLBACK (on_layout), NULL);
  build_window ();

  input = g_io_channel_unix_new (0);
  g_io_add_watch (input, G_IO_IN | G_IO_HUP | G_IO_ERR, on_command, NULL);
  loop = g_main_loop_new (NULL, FALSE);
  printf ("READY\n");
  fflush (stdout);
  g_main_loop_run (loop);
  return 0;
}
