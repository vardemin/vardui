# JNI: the Rust core calls VardHost.call(String, String) by name and binds VardNative natives.
-keep class dev.vard.host.VardNative { *; }
-keep class dev.vard.host.VardHost { public java.lang.String call(java.lang.String, java.lang.String); }
# Module libraries are instantiated from manifest meta-data by class name.
-keep class * implements dev.vard.host.VardModule { <init>(); }
