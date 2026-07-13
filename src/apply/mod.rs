pub mod gnome;
pub mod gtk_css;
pub mod reload;
pub mod wallpaper;

#[derive(Debug, Clone, Copy, Default)]
pub struct ApplyFlags {
    pub skip_apply: bool,
    pub skip_gnome: bool,
    pub skip_icons: bool,
    pub skip_reload: bool,
    pub skip_wallpaper: bool,
}
