scalable_dir = 'hicolor' / 'scalable' / 'apps'
if profile == 'development'
  scalable_icon = scalable_dir / 'de.f1ori.longlens.Devel.svg'
else
  scalable_icon = scalable_dir / 'de.f1ori.longlens.svg'
endif
install_data(
  scalable_icon,
  install_dir: get_option('datadir') / 'icons' / scalable_dir,
  rename: application_id + '.svg',
)

symbolic_dir = 'hicolor' / 'symbolic' / 'apps'
install_data(
  symbolic_dir / 'de.f1ori.longlens-symbolic.svg',
  install_dir: get_option('datadir') / 'icons' / symbolic_dir,
  rename: application_id + '-symbolic.svg',
)
