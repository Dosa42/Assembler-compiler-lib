# Licenses and origins

The selected Brokkr source and engine adaptation are distributed under GPL-3.0-or-later. Original notices remain in `upstream/original`; the GPL text is retained as `Brokkr-GPL-3.0.txt`. The source comes from [Brokkr v2.5.0](https://github.com/Gabriel2392/brokkr-flash/tree/44afa09b9e6c523cfc564810c3f3c1c083e725b7), commit `44afa09b9e6c523cfc564810c3f3c1c083e725b7`. The source release matches the official Android APK release identity; this package is rebuilt source, not an extracted APK library.

| Component | License | Retained material |
| --- | --- | --- |
| Brokkr core, Android transport and formatting/logging shims | GPL-3.0-or-later | Original source notices and GPL text |
| LZ4 and xxHash | BSD-2-Clause | Full original source and header notices |
| MD5 implementation | Public domain | Full original source notice |
| libusb 1.0.30 | LGPL-2.1-or-later | Shared library, license, upstream release archive and pinned Termux build recipe/patches |
| Termux libc++ package 30, Android NDK r30 runtime | Installed package declares NCSA; LLVM runtime notices include Apache-2.0 with LLVM exceptions | Shared library, installed copyright, supplemental libc++/libc++abi notices and pinned Termux recipe |
| USB broker/client adaptation | MIT | `usb/LICENSE` and source notices |

The dependency binaries come from installed Termux packages and only their ELF runpaths are changed. Their source-to-binary reproducibility has not been independently established. [Dependency provenance](../upstream/dependencies/provenance.json) records source URLs and hashes. The libusb source can be rebuilt with its retained recipe/patches; the dynamic library remains replaceable. Android NDK/toolchain archives are not bundled. Android system libraries are host APIs and are not copied.

The separate CI static target links Alpine/musl, GCC libstdc++ and Alpine's libusb archive instead of the Termux shared libraries. That archive must retain its actual build dependency versions and notices: musl MIT, GCC runtime GPL-3.0 with the GCC Runtime Library Exception, and libusb LGPL-2.1-or-later. The CI package includes the Brokkr static API archive and source for relinking. Bionic dependency hashes and execution evidence do not describe that static output.
