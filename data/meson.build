conf = configuration_data()
conf.set('APP_ID', application_id)
conf.set('APP_PATH', app_path)

desktop_in = configure_file(
  input: 'de.f1ori.longlens.desktop.in.in',
  output: 'de.f1ori.longlens.desktop.in',
  configuration: conf,
)

desktop_file = i18n.merge_file(
        input: desktop_in,
       output: application_id + '.desktop',
         type: 'desktop',
       po_dir: '../po',
      install: true,
  install_dir: get_option('datadir') / 'applications'
)

desktop_utils = find_program('desktop-file-validate', required: false)
if desktop_utils.found()
  test('Validate desktop file', desktop_utils, args: [desktop_file])
endif

metainfo_in = configure_file(
  input: 'de.f1ori.longlens.metainfo.xml.in.in',
  output: 'de.f1ori.longlens.metainfo.xml.in',
  configuration: conf,
)

appstream_file = i18n.merge_file(
        input: metainfo_in,
       output: application_id + '.metainfo.xml',
       po_dir: '../po',
      install: true,
  install_dir: get_option('datadir') / 'metainfo'
)

appstreamcli = find_program('appstreamcli', required: false, disabler: true)
test('Validate appstream file', appstreamcli,
     args: ['validate', '--no-net', '--explain', appstream_file])

gschema_file = configure_file(
  input: 'de.f1ori.longlens.gschema.xml.in',
  output: application_id + '.gschema.xml',
  configuration: conf,
  install: true,
  install_dir: get_option('datadir') / 'glib-2.0' / 'schemas',
)

compile_schemas = find_program('glib-compile-schemas', required: false, disabler: true)
test('Validate schema file',
     compile_schemas,
     args: ['--strict', '--dry-run', meson.current_build_dir()])

service_conf = configuration_data()
service_conf.set('bindir', get_option('prefix') / get_option('bindir'))
service_conf.set('APP_ID', application_id)
configure_file(
  input: 'de.f1ori.longlens.service.in',
  output: application_id + '.service',
  configuration: service_conf,
  install_dir: get_option('datadir') / 'dbus-1' / 'services'
)

search_provider_conf = configuration_data()
search_provider_conf.set('APP_ID', application_id)
configure_file(
  input: 'de.f1ori.longlens.search-provider.ini.in',
  output: application_id + '.search-provider.ini',
  configuration: search_provider_conf,
  install: true,
  install_dir: get_option('datadir') / 'gnome-shell' / 'search-providers',
)

pkgdatadir = get_option('prefix') / get_option('datadir') / meson.project_name()

blueprints = custom_target('blueprints',
  input: files(
    'ui/destination_dialog.blp',
    'ui/destination_row.blp',
    'ui/destinations_page.blp',
    'ui/password_dialog.blp',
    'ui/window.blp',
  ),
  output: '.',
  command: [find_program('blueprint-compiler'), 'batch-compile', '@OUTPUT@', '@CURRENT_SOURCE_DIR@', '@INPUT@'],
)

gresource_xml = configure_file(
  input: 'de.f1ori.longlens.gresource.xml.in',
  output: 'de.f1ori.longlens.gresource.xml',
  configuration: conf,
)

gnome.compile_resources('longlens',
  gresource_xml,
  gresource_bundle: true,
  install: true,
  install_dir: pkgdatadir,
  dependencies: [blueprints, appstream_file],
  source_dir: [meson.current_source_dir(), meson.current_build_dir()],
)

subdir('icons')
