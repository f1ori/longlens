project('longlens', 'rust',
          version: '0.4.0',
    meson_version: '>= 1.1.0',
  default_options: [ 'warning_level=2', 'werror=false', ],
)

i18n = import('i18n')
gnome = import('gnome')

profile = get_option('profile')
if profile == 'development'
  application_id = 'de.f1ori.longlens.Devel'
  app_path = '/de/f1ori/longlens/Devel/'
  app_name = 'Long Lens (Devel)'
else
  application_id = 'de.f1ori.longlens'
  app_path = '/de/f1ori/longlens/'
  app_name = 'Long Lens'
endif


subdir('data')
subdir('src')
subdir('po')

gnome.post_install(
     glib_compile_schemas: true,
    gtk_update_icon_cache: true,
  update_desktop_database: true,
)
