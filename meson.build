# meson is the single entry point: it builds the 70 CLAN programs (1990s C++),
# the translation catalogues, and — by driving cargo — the Rust desktop app.
project('talkbank', ['cpp'],
  version: '0.1.1',
  meson_version: '>= 1.0',
  default_options: ['warning_level=1', 'buildtype=release'])

root      = meson.current_source_dir()
build_dir = meson.current_build_dir()
gettext_package = 'talkbank'

# ---------------------------------------------------------------------------
# CLAN programs (1990s C++)
#
#  -std=gnu++98 -fpermissive : the sources do not compile under a modern g++
#                              default standard
#  -I compat                 : glibc removed <termio.h>
#  -DDEPDIR                  : location of the lib directory, per src/0README.TXT
# ---------------------------------------------------------------------------
# DEPDIR is compiled in, and the CLAN programs have no way to look it up at run
# time. It defaults to the source tree, which is what makes `build/freq` work
# straight away; packagers should point it at the installed copy instead:
#
#   meson setup build -Ddepdir=/usr/local/share/talkbank/lib
depdir = get_option('depdir')
if depdir == ''
  depdir = root / 'lib'
endif

clan_args = [
  '-DUNX', '-std=gnu++98', '-fpermissive', '-w',
  '-I' + root / 'compat',
  '-I' + root / 'src',
  '-DDEPDIR="' + depdir + '/"',
]

# The 70 programs go into a private directory, **not** into bin/.
#
# Three of them are named after standard Unix commands — `uniq` (coreutils),
# `script` (util-linux) and `gem` (RubyGems) — and others (`indent`, `repeat`,
# `post`, `dist`, `lines`, `check`) collide on some systems. Putting them on the
# PATH shadows those commands for everything the user runs, which broke shell
# startup files that call `uniq`. The app locates them by itself; anyone who
# wants them in a shell appends this directory to PATH deliberately, so the
# system commands keep winning. See docs/development.md.
clan_bindir = get_option('libexecdir') / 'talkbank'

clancore = static_library('clancore',
  files('src/cutt.cpp', 'src/c_ver.cpp', 'src/stringparser.cpp',
        'src/fontconvert.cpp', 'src/cutt-xml.cpp'),
  cpp_args: clan_args)

m_dep = meson.get_compiler('cpp').find_library('m', required: false)

# command name -> sources (relative to src/)
simple_programs = {
  'chains'    : ['chains.cpp'],        'chat2elan' : ['Chat2Elan.cpp'],
  'chat2praat': ['Chat2Praat.cpp'],    'check'     : ['check.cpp'],
  'chip'      : ['chip.cpp', 'chip_utl.cpp'],
  'chstring'  : ['chstring.cpp'],      'codes'     : ['codes-p.cpp'],
  'combo'     : ['combo.cpp'],         'combtier'  : ['combtier.cpp'],
  'cooccur'   : ['cooccur.cpp'],       'dataclean' : ['DataCleanUp.cpp'],
  'dates'     : ['dates.cpp'],         'delim'     : ['delim.cpp'],
  'dss'       : ['dss.cpp'],
  'elan2chat' : ['Elan2Chat.cpp'],     'eval'      : ['eval.cpp'],
  'eval-d'    : ['eval-d.cpp'],        'fixbullets': ['FixBullets.cpp'],
  'fixit'     : ['fixit.cpp'],         'flo'       : ['flo.cpp'],
  'flucalc'   : ['flucalc.cpp'],       'freq'      : ['freq.cpp'],
  'freqpos'   : ['freqpos.cpp'],       'gem'       : ['gem.cpp'],
  'gemfreq'   : ['gemfreq.cpp'],       'gemlist'   : ['gemlist.cpp'],
  'indent'    : ['indent.cpp'],        'ipsyn'     : ['ipsyn.cpp'],
  'keymap'    : ['keymap.cpp'],        'kwal'      : ['kwal.cpp'],
  'lena2chat' : ['Lena2Chat.cpp'],     'lines'     : ['lines.cpp'],
  'lipp2chat' : ['lippUTF2Chat.cpp'],  'longtier'  : ['longtier.cpp'],
  'lowcase'   : ['lowcase.cpp'],       'makemod'   : ['makemod.cpp'],
  'maxwd'     : ['maxwd.cpp'],         'mlt'       : ['mlt.cpp', 'mllib.cpp'],
  'mlu'       : ['mlu.cpp', 'mllib.cpp'],
  'modrep'    : ['modrep.cpp'],        'mortable'  : ['mortable.cpp'],
  'ort'       : ['ort.cpp'],           'phonfreq'  : ['phonfreq.cpp'],
  'play2chat' : ['Play2Chat.cpp'],     'postmortem': ['postmortem.cpp'],
  'praat2chat': ['Praat2Chat.cpp'],    'quotes'    : ['Quotes.cpp'],
  'rely'      : ['rely.cpp'],          'repeat'    : ['repeat.cpp'],
  'retrace'   : ['retrace.cpp'],       'salt2chat' : ['salt2chat.cpp'],
  'script'    : ['script.cpp'],        'srt2chat'  : ['Srt2Chat.cpp'],
  'sugar'     : ['sugar.cpp'],         'text2chat' : ['text2chat.cpp'],
  'tierorder' : ['tierorder.cpp'],     'timedur'   : ['timedur.cpp'],
  'trnfix'    : ['TrnFix.cpp'],        'uniq'      : ['uniq.cpp'],
  'wdlen'     : ['wdlen.cpp'],
}

foreach name, srcs : simple_programs
  paths = []
  foreach s : srcs
    paths += root / 'src' / s
  endforeach
  executable(name, paths,
    link_with: clancore, cpp_args: clan_args, dependencies: m_dep, install: true, install_dir: clan_bindir)
endforeach

# "dist" is reserved by meson, both as a target name and as an output name: we
# build it as clan-dist and put a copy next to it under the name CLAN expects.
sh = find_program('sh')
dist_exe = executable('clan-dist', root / 'src/dist.cpp',
  link_with: clancore, cpp_args: clan_args, dependencies: m_dep, install: true, install_dir: clan_bindir)
custom_target('dist-alias',
  input: dist_exe, output: 'dist.stamp',
  command: [sh, '-c', 'cp "$1" "$(dirname "$2")/dist" && touch "$2"', 'sh', '@INPUT@', '@OUTPUT@'],
  build_by_default: true)
meson.add_install_script(sh, '-c',
  'cd "$MESON_INSTALL_DESTDIR_PREFIX/' + clan_bindir + '" && cp -f clan-dist dist')

vocd_src = files('src/vocd/args.cpp', 'src/vocd/speaker.cpp', 'src/vocd/tokens.cpp',
                 'src/vocd/filter.cpp', 'src/vocd/dcompute.cpp', 'src/vocd/vocd.cpp')
executable('vocd', vocd_src,
  link_with: clancore, cpp_args: clan_args + ['-I' + root / 'src/vocd'],
  dependencies: m_dep, install: true, install_dir: clan_bindir)

executable('kideval',
  [files('src/kideval.cpp', 'src/dss.cpp', 'src/ipsyn.cpp'), vocd_src],
  link_with: clancore,
  cpp_args: clan_args + ['-I' + root / 'src/vocd', '-DKIDEVAL_LIB'],
  dependencies: m_dep, install: true, install_dir: clan_bindir)

executable('mor',
  files('src/mor/arules.cpp', 'src/mor/crules.cpp', 'src/mor/drules.cpp',
        'src/mor/globals.cpp', 'src/mor/lex-trie.cpp', 'src/mor/morlib.cpp',
        'src/mor/mor.cpp'),
  link_with: clancore, cpp_args: clan_args + ['-I' + root / 'src/mor'],
  dependencies: m_dep, install: true, install_dir: clan_bindir)

executable('megrasp',
  files('src/megrasp/megrasp.cpp', 'src/megrasp/maxent.cpp', 'src/megrasp/blmvm.cpp'),
  link_with: clancore, cpp_args: clan_args + ['-I' + root / 'src/megrasp'],
  dependencies: m_dep, install: true, install_dir: clan_bindir)

# The POST family does not use the cutt library: it has its own main and runtime.
post_common = files(
  'src/post/anaalgo.cpp', 'src/post/atom.cpp', 'src/post/brillanx.cpp',
  'src/post/brilltagger.cpp', 'src/post/compoundlib.cpp', 'src/post/database.cpp',
  'src/post/hashbsc.cpp', 'src/post/hashc.cpp', 'src/post/input.cpp',
  'src/post/mortags.cpp', 'src/post/msg.cpp', 'src/post/rules.cpp',
  'src/post/splitchat.cpp', 'src/post/storage.cpp', 'src/post/tags.cpp',
  'src/post/workspace.cpp')
post_args = clan_args + ['-I' + root / 'src/post']

post_programs = {
  'compound'    : ['compound.cpp'],
  'post'        : ['postana.cpp', 'post.cpp'],
  'postlist'    : ['trainproc.cpp', 'postlist.cpp'],
  'postmodrules': ['postmodrules.cpp'],
  'posttrain'   : ['trainproc.cpp', 'trainbrill.cpp', 'contextrl.cpp', 'darray.cpp',
                   'registry.cpp', 'memory.cpp', 'posttrain.cpp'],
}
foreach name, srcs : post_programs
  paths = []
  foreach s : srcs
    paths += root / 'src/post' / s
  endforeach
  executable(name, [post_common, paths],
    cpp_args: post_args, dependencies: m_dep, install: true, install_dir: clan_bindir)
endforeach

# ---------------------------------------------------------------------------
# Translations
#
# The <localedir>/<lang>/LC_MESSAGES/talkbank.mo layout is created inside the
# build directory, so the app works without being installed.
# ---------------------------------------------------------------------------
msgfmt = find_program('msgfmt')
languages = ['it', 'en']

mo_files = []
foreach lang : languages
  mo_files += custom_target('mo-' + lang,
    input: 'po' / lang + '.po',
    output: lang + '.mo',
    command: [msgfmt, '--check-format', '-o', '@OUTPUT@', '@INPUT@'],
    build_by_default: true)
endforeach

locale_tree = custom_target('locale-tree',
  input: mo_files,
  output: 'locale.stamp',
  command: [find_program('sh'), '-c',
    'set -e; i=0; for l in ' + ' '.join(languages) + '; do ' +
    '  mkdir -p "$1/locale/$l/LC_MESSAGES"; ' +
    '  cp "$1/$l.mo" "$1/locale/$l/LC_MESSAGES/' + gettext_package + '.mo"; ' +
    'done; touch "$1/locale.stamp"',
    'sh', build_dir],
  build_by_default: true)

# The desktop entry carries the absolute path to the binary: a `~/.local` install
# is not on the PATH of every desktop session, and a launcher that silently does
# nothing is worse than no launcher.
desktop = configure_file(
  input: 'data/org.talkbank.TalkBank.desktop.in',
  output: 'org.talkbank.TalkBank.desktop',
  configuration: {'BINDIR': get_option('prefix') / get_option('bindir')},
  install: true,
  install_dir: get_option('datadir') / 'applications')

desktop_validate = find_program('desktop-file-validate', required: false)
if desktop_validate.found()
  test('desktop-file', desktop_validate, args: [desktop])
endif

install_data('data/icons/hicolor/128x128/apps/org.talkbank.TalkBank.png',
  install_dir: get_option('datadir') / 'icons/hicolor/128x128/apps')

# Without these the entry sits on disk and the launcher never sees it: the
# desktop database and the icon cache have to be told. `gnome.post_install()`
# runs the right tools and skips itself under DESTDIR, where they make no sense.
gnome = import('gnome')
gnome.post_install(
  gtk_update_icon_cache: true,
  update_desktop_database: true)

# The start page links to the guide, so it has to be installed next to the app.
install_data('docs/guide.md', 'README.md',
  install_dir: get_option('datadir') / 'doc/talkbank')

# The language data the CLAN programs read (grammars, DSS and IPSyn tables,
# KIDEVAL reference values). See the note on DEPDIR above.
install_subdir('lib', install_dir: get_option('datadir') / 'talkbank')

# The compiled catalogues. `locale_tree` already lays them out as
# <lang>/LC_MESSAGES/talkbank.mo inside the build directory; an install script
# copies that tree, because the directory does not exist at configure time.
meson.add_install_script(sh, '-c',
  'set -e; for l in ' + ' '.join(languages) + '; do ' +
  '  d="$MESON_INSTALL_DESTDIR_PREFIX/' + get_option('datadir') + '/locale/$l/LC_MESSAGES"; ' +
  '  mkdir -p "$d"; ' +
  '  cp "$1/locale/$l/LC_MESSAGES/' + gettext_package + '.mo" "$d/"; ' +
  'done',
  'sh', build_dir)

alias_target('translations', locale_tree)

# ---------------------------------------------------------------------------
# The desktop app (Rust)
#
# cargo tracks its own dependencies far better than meson could, so the target
# is always considered stale and cargo decides whether there is work to do.
# `console: true` lets its progress reach the terminal.
# ---------------------------------------------------------------------------
cargo = find_program('cargo', required: get_option('app'))

if cargo.found()
  cargo_profile = get_option('buildtype').startswith('debug') ? 'debug' : 'release'
  app = custom_target('talkbank',
    output: 'talkbank',
    command: [find_program(root / 'tools/build-app.sh'),
              root, '@OUTPUT@', cargo_profile],
    build_by_default: true,
    console: true,
    build_always_stale: true,
    install: true,
    install_dir: get_option('bindir'))
endif

summary({
  'CLAN programs' : simple_programs.keys().length() + post_programs.keys().length() + 4,
  'desktop app'   : cargo.found(),
  'languages'     : ', '.join(languages),
  'lib directory' : root / 'lib',
})
