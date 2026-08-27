'use strict';
gg = { m: function(g) {
var o = 1;
switch (g) {
case 4062:
case 2748:
o = 0; break;
case 33:
o = 1; break;
}
return o;
},
s: function(h) { var m = /(..)(.)$/.exec(h); return parseInt(m[2]+m[1], 16).toString(10); },
b: '1786694402/'
};
