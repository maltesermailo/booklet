# Wayland and KMS

The compositor talks to hardware through DRM/KMS, not a display server. Every plane and CRTC is a kernel object.

## Atomic commits

Stage a full display state, commit in one ioctl — it applies whole or fails whole.
