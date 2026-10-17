/* A GTK host for the gnist-gio layout tests.
 *
 * Built against GTK 3 or GTK 4. With LAYOUT_HOST_PATCHED=1 it gives GtkSettings
 * a "gtk-herda-layout" string property, standing in for the patched toolkit;
 * otherwise GtkSettings is upstream's. It loads gnist-gio the way an
 * application does, through GIO_EXTRA_MODULES, and reports on stdout:
 *
 *   NOTIFY <layout>          GtkSettings:gtk-herda-layout changed
 *   WINDOW <n> <layout>      a window built now would read this layout
 *
 * Commands on stdin: "window" builds a window, "quit" (or EOF) exits.
 */
#include <gtk/gtk.h>
#include <stdio.h>
#include <string.h>

#define LAYOUT_PROPERTY "gtk-herda-layout"

static GMainLoop *loop;
static int windows;

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
  gtk_window_set_child (GTK_WINDOW (window), gtk_label_new ("layout host"));
  gtk_window_present (GTK_WINDOW (window));
#else
  GtkWidget *window = gtk_window_new (GTK_WINDOW_TOPLEVEL);
  gtk_container_add (GTK_CONTAINER (window), gtk_label_new ("layout host"));
  gtk_widget_show_all (window);
#endif
  printf ("WINDOW %d %s\n", ++windows, layout);
  fflush (stdout);
  g_free (layout);
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

  stand_in_layout = g_strdup ("native");
  upstream_set = klass->set_property;
  upstream_get = klass->get_property;
  klass->set_property = stand_in_set;
  klass->get_property = stand_in_get;
  g_object_class_install_property (klass, PROP_LAYOUT,
                                   g_param_spec_string (LAYOUT_PROPERTY, NULL, NULL, "native",
                                                        G_PARAM_READWRITE | G_PARAM_STATIC_STRINGS));
}
#else
static void
patch_settings (void)
{
  G_GNUC_BEGIN_IGNORE_DEPRECATIONS
  gtk_settings_install_property (g_param_spec_string (LAYOUT_PROPERTY, NULL, NULL, "native",
                                                      G_PARAM_READWRITE | G_PARAM_STATIC_STRINGS));
  G_GNUC_END_IGNORE_DEPRECATIONS
}
#endif

int
main (int argc, char **argv)
{
  gboolean patched = g_strcmp0 (g_getenv ("LAYOUT_HOST_PATCHED"), "1") == 0;
  gboolean early = g_strcmp0 (g_getenv ("LAYOUT_HOST_EARLY_MODULES"), "1") == 0;
  GIOChannel *input;

  /* Applications load GIO modules either before GTK is initialized (a
   * GApplication's registration) or after it; both orders must work. */
  if (early)
    g_vfs_get_default ();
  if (patched)
    patch_settings ();
#if GTK_MAJOR_VERSION >= 4
  gtk_init ();
#else
  gtk_init (&argc, &argv);
#endif
  if (!early)
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
