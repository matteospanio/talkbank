/* ponytail: glibc removed <termio.h>. cutt.cpp only uses struct termio +
   TCGETA as an isatty() test, so aliasing them onto POSIX termios is enough.
   Added via -Icompat so upstream sources stay untouched. */
#ifndef CLAN_COMPAT_TERMIO_H
#define CLAN_COMPAT_TERMIO_H
#include <termios.h>
#include <sys/ioctl.h>
#define termio termios
#ifndef TCGETA
#define TCGETA TCGETS
#endif
#endif
