%%start
std = namespace std "bootstrap.w";

char ascii = 'A';
std.utf8 out_ascii = std.format(ascii);
std.print(out_ascii);
std.print("\n");
char latin = '\u{E9}';
std.utf8 out_latin = std.format(latin);
std.print(out_latin);
std.print("\n");
char euro = '\u{20AC}';
std.utf8 out_euro = std.format(euro);
std.print(out_euro);
std.print("\n");
char top = '\u{10FFFF}';
std.utf8 out_top = std.format(top);
std.print(out_top);
std.print("\n");
std.exit(0);
%%end
