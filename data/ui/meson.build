blueprint_compiler = find_program('blueprint-compiler')

blueprints = []
foreach name : [
  'destination_dialog',
  'destination_row',
  'destinations_page',
  'password_dialog',
  'window',
]
  blueprints += custom_target(name + '.ui',
    input: name + '.blp',
    output: name + '.ui',
    command: [blueprint_compiler, 'compile', '--output', '@OUTPUT@', '@INPUT@'],
  )
endforeach
