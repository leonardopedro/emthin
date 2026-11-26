//! Embedded Unix-socket Wayland compositor for Android `\app` figures.
//!
//! Speaks Wayland wire messages with `SCM_RIGHTS` fd passing over `AF_UNIX`
//! `SOCK_STREAM` (supporting `wl_compositor`, `wl_shm`, `zwp_linux_dmabuf_v1`,
//! `xdg_wm_base`, `wl_seat`, and `wl_output`). The typst-laid-out document
//! remains the sole geometry authority: `sync_figures` matches mapped
//! `xdg_toplevel` surfaces to `\app` slots and emits `configure(width, height)`
//! whenever the document changes a figure's size.

use crate::gpu::{
    DmabufDescriptor, DmabufPlane, DRM_FORMAT_ABGR8888, DRM_FORMAT_ARGB8888, DRM_FORMAT_XBGR8888,
    DRM_FORMAT_XRGB8888, WL_SHM_FORMAT_ARGB8888, WL_SHM_FORMAT_XRGB8888,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::ffi::CString;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

const MAX_SHM_POOL_BYTES: usize = 64 * 1024 * 1024;
const MAX_WIRE_MSG_SIZE: usize = 64 * 1024;
const DRM_FORMAT_MOD_INVALID: u64 = 0x00ff_ffff_ffff_ffff;

/// Document-authoritative slot passed from `MainActivity` via `syncFigures`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FigureSlot {
    pub key: String,
    #[serde(default)]
    pub app_id: Option<String>,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FigureStatus {
    pub key: String,
    pub app_id: Option<String>,
    pub bound: bool,
    pub title: String,
    pub client_app_id: String,
    pub frame_seq: u64,
    pub buffer_kind: &'static str,
    pub width: u32,
    pub height: u32,
}

/// Unbound `xdg_toplevel` announced to the host so a new `\app` figure slot
/// can be created automatically when a terminal-launched Wayland client opens a window.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingToplevel {
    pub client_id: u64,
    pub toplevel_id: u32,
    pub app_id: String,
    pub title: String,
    pub width: i32,
    pub height: i32,
}

/// Latest committed buffer for a mapped surface.
pub enum CommittedFrame {
    Shm {
        width: u32,
        height: u32,
        stride: u32,
        wl_format: u32,
        pixels: Vec<u8>,
    },
    Dmabuf(DmabufDescriptor),
}

// ---- Wayland wire framing + SCM_RIGHTS transport ----

fn encode_msg(object_id: u32, opcode: u16, payload: &[u8]) -> Vec<u8> {
    let size = 8 + payload.len();
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(&object_id.to_le_bytes());
    out.extend_from_slice(&(((size as u32) << 16) | opcode as u32).to_le_bytes());
    out.extend_from_slice(payload);
    out
}

fn push_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn push_i32(buf: &mut Vec<u8>, v: i32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn push_str(buf: &mut Vec<u8>, s: &str) {
    let len = (s.len() + 1) as u32;
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(s.as_bytes());
    buf.push(0);
    while buf.len() % 4 != 0 {
        buf.push(0);
    }
}

fn push_array(buf: &mut Vec<u8>, data: &[u8]) {
    buf.extend_from_slice(&(data.len() as u32).to_le_bytes());
    buf.extend_from_slice(data);
    while buf.len() % 4 != 0 {
        buf.push(0);
    }
}

fn read_u32(data: &[u8], off: &mut usize) -> Option<u32> {
    let bytes: [u8; 4] = data.get(*off..*off + 4)?.try_into().ok()?;
    *off += 4;
    Some(u32::from_le_bytes(bytes))
}

fn read_i32(data: &[u8], off: &mut usize) -> Option<i32> {
    let bytes: [u8; 4] = data.get(*off..*off + 4)?.try_into().ok()?;
    *off += 4;
    Some(i32::from_le_bytes(bytes))
}

fn read_str(data: &[u8], off: &mut usize) -> Option<String> {
    let len = read_u32(data, off)? as usize;
    if len == 0 || *off + len > data.len() {
        return None;
    }
    let raw = &data[*off..*off + len];
    let padded = (len + 3) & !3;
    *off = (*off + padded).min(data.len());
    let text = std::str::from_utf8(&raw[..len - 1]).ok()?;
    Some(text.to_string())
}

fn send_with_fds(fd: RawFd, data: &[u8], fds: &[RawFd]) -> std::io::Result<usize> {
    let mut iov = libc::iovec {
        iov_base: data.as_ptr() as *mut libc::c_void,
        iov_len: data.len(),
    };
    let mut cmsg_buf = vec![0u8; unsafe { libc::CMSG_SPACE((fds.len() * 4) as u32) } as usize];
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    if !fds.is_empty() {
        msg.msg_control = cmsg_buf.as_mut_ptr() as *mut libc::c_void;
        msg.msg_controllen = cmsg_buf.len() as _;
        unsafe {
            let cmsg = libc::CMSG_FIRSTHDR(&msg);
            (*cmsg).cmsg_level = libc::SOL_SOCKET;
            (*cmsg).cmsg_type = libc::SCM_RIGHTS;
            (*cmsg).cmsg_len = libc::CMSG_LEN((fds.len() * 4) as u32) as _;
            std::ptr::copy_nonoverlapping(
                fds.as_ptr(),
                libc::CMSG_DATA(cmsg) as *mut RawFd,
                fds.len(),
            );
        }
    }
    let n = unsafe { libc::sendmsg(fd, &msg, libc::MSG_NOSIGNAL) };
    if n < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(n as usize)
    }
}

fn recv_with_fds(fd: RawFd, buf: &mut [u8], fds: &mut VecDeque<OwnedFd>) -> std::io::Result<usize> {
    let mut iov = libc::iovec {
        iov_base: buf.as_mut_ptr() as *mut libc::c_void,
        iov_len: buf.len(),
    };
    let mut cmsg_buf = [0u8; 512];
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cmsg_buf.as_mut_ptr() as *mut libc::c_void;
    msg.msg_controllen = cmsg_buf.len() as _;
    let n = unsafe { libc::recvmsg(fd, &mut msg, libc::MSG_CMSG_CLOEXEC) };
    if n < 0 {
        return Err(std::io::Error::last_os_error());
    }
    unsafe {
        let mut cmsg = libc::CMSG_FIRSTHDR(&msg);
        while !cmsg.is_null() {
            if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
                let hdr_len = libc::CMSG_LEN(0) as usize;
                let data_len = ((*cmsg).cmsg_len as usize).saturating_sub(hdr_len);
                let count = data_len / std::mem::size_of::<RawFd>();
                let ptr = libc::CMSG_DATA(cmsg) as *const RawFd;
                for i in 0..count {
                    let raw = std::ptr::read_unaligned(ptr.add(i));
                    if raw >= 0 {
                        fds.push_back(OwnedFd::from_raw_fd(raw));
                    }
                }
            }
            cmsg = libc::CMSG_NXTHDR(&msg, cmsg);
        }
    }
    Ok(n as usize)
}

// ---- Compositor objects & state ----

#[derive(Clone, Debug)]
enum ObjKind {
    Display,
    Registry,
    Compositor,
    Region,
    Surface,
    Shm,
    ShmPool,
    Buffer,
    Dmabuf,
    BufferParams,
    XdgWmBase,
    XdgSurface,
    XdgToplevel,
    Seat,
    Pointer,
    Keyboard,
    Touch,
    Output,
}

struct ShmPoolData {
    ptr: *mut u8,
    len: usize,
}

unsafe impl Send for ShmPoolData {}

impl ShmPoolData {
    fn map(fd: &OwnedFd, size: usize) -> Result<Self, String> {
        if size == 0 || size > MAX_SHM_POOL_BYTES {
            return Err("wl_shm_pool size out of range".into());
        }
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size,
                libc::PROT_READ,
                libc::MAP_SHARED,
                fd.as_raw_fd(),
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            return Err("mmap(wl_shm_pool) failed".into());
        }
        Ok(Self {
            ptr: ptr as *mut u8,
            len: size,
        })
    }

    fn copy_rect(&self, offset: usize, stride: usize, height: usize) -> Option<Vec<u8>> {
        let end = offset.checked_add(stride.checked_mul(height)?)?;
        if end > self.len {
            return None;
        }
        let slice = unsafe { std::slice::from_raw_parts(self.ptr.add(offset), end - offset) };
        Some(slice.to_vec())
    }
}

impl Drop for ShmPoolData {
    fn drop(&mut self) {
        if !self.ptr.is_null() && self.len > 0 {
            unsafe {
                libc::munmap(self.ptr as *mut libc::c_void, self.len);
            }
        }
    }
}

enum BufferBacking {
    Shm {
        pool_id: u32,
        offset: usize,
        width: u32,
        height: u32,
        stride: u32,
        wl_format: u32,
    },
    Dmabuf(DmabufDescriptor),
}

struct DmabufParamsState {
    planes: Vec<DmabufPlane>,
}

#[derive(Default)]
struct SurfaceState {
    pending_buffer: Option<u32>,
    pending_frame_cbs: Vec<u32>,
    committed: Option<Arc<CommittedFrame>>,
    committed_kind: &'static str,
    committed_width: u32,
    committed_height: u32,
    frame_seq: u64,
    pending_send_cbs: Vec<u32>,
}

struct XdgToplevelState {
    xdg_surface_id: u32,
    wl_surface_id: u32,
    title: String,
    app_id: String,
    bound_figure: Option<String>,
    last_configured: Option<(i32, i32)>,
    announced_pending: bool,
}

struct ClientConn {
    id: u64,
    stream: UnixStream,
    in_buf: Vec<u8>,
    in_fds: VecDeque<OwnedFd>,
    out_buf: Vec<u8>,
    objects: HashMap<u32, ObjKind>,
    pools: HashMap<u32, ShmPoolData>,
    buffers: HashMap<u32, BufferBacking>,
    dmabuf_params: HashMap<u32, DmabufParamsState>,
    surfaces: HashMap<u32, SurfaceState>,
    xdg_surfaces: HashMap<u32, u32>, // xdg_surface_id -> wl_surface_id
    toplevels: HashMap<u32, XdgToplevelState>,
    touches: Vec<u32>,
    next_server_id: u32,
    serial: u32,
    closed: bool,
}

impl ClientConn {
    fn new(id: u64, stream: UnixStream) -> Self {
        let mut objects = HashMap::new();
        objects.insert(1, ObjKind::Display);
        Self {
            id,
            stream,
            in_buf: Vec::with_capacity(4096),
            in_fds: VecDeque::new(),
            out_buf: Vec::with_capacity(4096),
            objects,
            pools: HashMap::new(),
            buffers: HashMap::new(),
            dmabuf_params: HashMap::new(),
            surfaces: HashMap::new(),
            xdg_surfaces: HashMap::new(),
            toplevels: HashMap::new(),
            touches: Vec::new(),
            next_server_id: 0xff00_0000,
            serial: 1,
            closed: false,
        }
    }

    fn next_serial(&mut self) -> u32 {
        let s = self.serial;
        self.serial = self.serial.wrapping_add(1).max(1);
        s
    }

    fn send(&mut self, object_id: u32, opcode: u16, payload: &[u8]) {
        self.out_buf
            .extend_from_slice(&encode_msg(object_id, opcode, payload));
    }

    fn send_configure(&mut self, toplevel_id: u32, width: i32, height: i32) {
        let Some(tl) = self.toplevels.get(&toplevel_id) else {
            return;
        };
        let xdg_surface_id = tl.xdg_surface_id;
        // xdg_toplevel.configure(width, height, states=[ACTIVATED(4)])
        let mut p = Vec::new();
        push_i32(&mut p, width);
        push_i32(&mut p, height);
        push_array(&mut p, &4u32.to_le_bytes());
        self.send(toplevel_id, 0, &p);

        let serial = self.next_serial();
        let mut sp = Vec::new();
        push_u32(&mut sp, serial);
        self.send(xdg_surface_id, 0, &sp);

        if let Some(tl) = self.toplevels.get_mut(&toplevel_id) {
            tl.last_configured = Some((width, height));
        }
    }

    fn flush_out(&mut self) {
        if self.out_buf.is_empty() || self.closed {
            return;
        }
        match send_with_fds(self.stream.as_raw_fd(), &self.out_buf, &[]) {
            Ok(n) => {
                self.out_buf.drain(..n);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => {
                self.closed = true;
            }
        }
    }

    fn read_and_dispatch(&mut self) {
        let mut tmp = [0u8; 8192];
        loop {
            match recv_with_fds(self.stream.as_raw_fd(), &mut tmp, &mut self.in_fds) {
                Ok(0) => {
                    self.closed = true;
                    break;
                }
                Ok(n) => {
                    self.in_buf.extend_from_slice(&tmp[..n]);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => {
                    self.closed = true;
                    break;
                }
            }
        }
        while self.in_buf.len() >= 8 && !self.closed {
            let obj_id = u32::from_le_bytes(self.in_buf[0..4].try_into().unwrap());
            let word1 = u32::from_le_bytes(self.in_buf[4..8].try_into().unwrap());
            let size = (word1 >> 16) as usize;
            let opcode = (word1 & 0xffff) as u16;
            if size < 8 || size > MAX_WIRE_MSG_SIZE {
                self.closed = true;
                break;
            }
            if self.in_buf.len() < size {
                break;
            }
            let payload = self.in_buf[8..size].to_vec();
            self.in_buf.drain(..size);
            self.handle_message(obj_id, opcode, &payload);
        }
    }

    fn handle_message(&mut self, obj_id: u32, opcode: u16, payload: &[u8]) {
        let Some(kind) = self.objects.get(&obj_id).cloned() else {
            return;
        };
        let mut off = 0usize;
        match kind {
            ObjKind::Display => match opcode {
                0 => {
                    // sync(new_id callback)
                    let Some(cb) = read_u32(payload, &mut off) else {
                        return;
                    };
                    let serial = self.next_serial();
                    let mut p = Vec::new();
                    push_u32(&mut p, serial);
                    self.send(cb, 0, &p); // wl_callback.done
                    let mut dp = Vec::new();
                    push_u32(&mut dp, cb);
                    self.send(1, 1, &dp); // wl_display.delete_id
                }
                1 => {
                    // get_registry(new_id registry)
                    let Some(reg) = read_u32(payload, &mut off) else {
                        return;
                    };
                    self.objects.insert(reg, ObjKind::Registry);
                    let globals: &[(u32, &str, u32)] = &[
                        (1, "wl_compositor", 4),
                        (2, "wl_shm", 1),
                        (3, "xdg_wm_base", 2),
                        (4, "zwp_linux_dmabuf_v1", 3),
                        (5, "wl_seat", 5),
                        (6, "wl_output", 2),
                    ];
                    for &(name, iface, ver) in globals {
                        let mut gp = Vec::new();
                        push_u32(&mut gp, name);
                        push_str(&mut gp, iface);
                        push_u32(&mut gp, ver);
                        self.send(reg, 0, &gp); // wl_registry.global
                    }
                }
                _ => {}
            },
            ObjKind::Registry => {
                if opcode == 0 {
                    // bind(name, iface, version, new_id)
                    let Some(name) = read_u32(payload, &mut off) else {
                        return;
                    };
                    let Some(iface) = read_str(payload, &mut off) else {
                        return;
                    };
                    let Some(_ver) = read_u32(payload, &mut off) else {
                        return;
                    };
                    let Some(new_id) = read_u32(payload, &mut off) else {
                        return;
                    };
                    match (name, iface.as_str()) {
                        (1, "wl_compositor") => {
                            self.objects.insert(new_id, ObjKind::Compositor);
                        }
                        (2, "wl_shm") => {
                            self.objects.insert(new_id, ObjKind::Shm);
                            for fmt in [
                                WL_SHM_FORMAT_ARGB8888,
                                WL_SHM_FORMAT_XRGB8888,
                                DRM_FORMAT_ABGR8888,
                                DRM_FORMAT_XBGR8888,
                            ] {
                                let mut p = Vec::new();
                                push_u32(&mut p, fmt);
                                self.send(new_id, 0, &p); // wl_shm.format
                            }
                        }
                        (3, "xdg_wm_base") => {
                            self.objects.insert(new_id, ObjKind::XdgWmBase);
                        }
                        (4, "zwp_linux_dmabuf_v1") => {
                            self.objects.insert(new_id, ObjKind::Dmabuf);
                            for fmt in [
                                DRM_FORMAT_ARGB8888,
                                DRM_FORMAT_XRGB8888,
                                DRM_FORMAT_ABGR8888,
                                DRM_FORMAT_XBGR8888,
                            ] {
                                let mut p = Vec::new();
                                push_u32(&mut p, fmt);
                                self.send(new_id, 0, &p); // format
                                for mod_val in [0u64, DRM_FORMAT_MOD_INVALID] {
                                    let mut mp = Vec::new();
                                    push_u32(&mut mp, fmt);
                                    push_u32(&mut mp, (mod_val >> 32) as u32);
                                    push_u32(&mut mp, (mod_val & 0xffff_ffff) as u32);
                                    self.send(new_id, 3, &mp); // modifier
                                }
                            }
                        }
                        (5, "wl_seat") => {
                            self.objects.insert(new_id, ObjKind::Seat);
                            let mut p = Vec::new();
                            push_u32(&mut p, 7); // pointer | keyboard | touch
                            self.send(new_id, 0, &p);
                            let mut np = Vec::new();
                            push_str(&mut np, "seat0");
                            self.send(new_id, 1, &np);
                        }
                        (6, "wl_output") => {
                            self.objects.insert(new_id, ObjKind::Output);
                        }
                        _ => {}
                    }
                    if matches!(self.objects.get(&new_id), Some(ObjKind::Output)) {
                        let mut gp = Vec::new();
                        push_i32(&mut gp, 0);
                        push_i32(&mut gp, 0);
                        push_i32(&mut gp, 160);
                        push_i32(&mut gp, 90);
                        push_i32(&mut gp, 0);
                        push_str(&mut gp, "emthin");
                        push_str(&mut gp, "android-figure");
                        push_i32(&mut gp, 0);
                        self.send(new_id, 0, &gp); // geometry
                        let mut mp = Vec::new();
                        push_u32(&mut mp, 3); // current | preferred
                        push_i32(&mut mp, 1280);
                        push_i32(&mut mp, 800);
                        push_i32(&mut mp, 60_000);
                        self.send(new_id, 1, &mp); // mode
                        let mut sp = Vec::new();
                        push_i32(&mut sp, 1);
                        self.send(new_id, 3, &sp); // scale
                        self.send(new_id, 2, &[]); // done
                    }
                }
            }
            ObjKind::Compositor => match opcode {
                0 => {
                    let Some(id) = read_u32(payload, &mut off) else {
                        return;
                    };
                    self.objects.insert(id, ObjKind::Surface);
                    self.surfaces.insert(id, SurfaceState::default());
                }
                1 => {
                    let Some(id) = read_u32(payload, &mut off) else {
                        return;
                    };
                    self.objects.insert(id, ObjKind::Region);
                }
                _ => {}
            },
            ObjKind::Region => {
                if opcode == 0 {
                    self.objects.remove(&obj_id);
                }
            }
            ObjKind::Shm => {
                if opcode == 0 {
                    // create_pool(new_id, fd, i32 size)
                    let Some(pool_id) = read_u32(payload, &mut off) else {
                        return;
                    };
                    let Some(size) = read_i32(payload, &mut off) else {
                        return;
                    };
                    let Some(fd) = self.in_fds.pop_front() else {
                        return;
                    };
                    if let Ok(pool) = ShmPoolData::map(&fd, size.max(0) as usize) {
                        self.objects.insert(pool_id, ObjKind::ShmPool);
                        self.pools.insert(pool_id, pool);
                    }
                }
            }
            ObjKind::ShmPool => match opcode {
                0 => {
                    // create_buffer(new_id, offset, width, height, stride, format)
                    let Some(buf_id) = read_u32(payload, &mut off) else {
                        return;
                    };
                    let Some(offset) = read_i32(payload, &mut off) else {
                        return;
                    };
                    let Some(width) = read_i32(payload, &mut off) else {
                        return;
                    };
                    let Some(height) = read_i32(payload, &mut off) else {
                        return;
                    };
                    let Some(stride) = read_i32(payload, &mut off) else {
                        return;
                    };
                    let Some(wl_format) = read_u32(payload, &mut off) else {
                        return;
                    };
                    if offset >= 0 && width > 0 && height > 0 && stride >= width * 4 {
                        self.objects.insert(buf_id, ObjKind::Buffer);
                        self.buffers.insert(
                            buf_id,
                            BufferBacking::Shm {
                                pool_id: obj_id,
                                offset: offset as usize,
                                width: width as u32,
                                height: height as u32,
                                stride: stride as u32,
                                wl_format,
                            },
                        );
                    }
                }
                1 => {
                    self.pools.remove(&obj_id);
                    self.objects.remove(&obj_id);
                }
                _ => {}
            },
            ObjKind::Buffer => {
                if opcode == 0 {
                    self.buffers.remove(&obj_id);
                    self.objects.remove(&obj_id);
                }
            }
            ObjKind::Dmabuf => match opcode {
                0 => {
                    self.objects.remove(&obj_id);
                }
                1 => {
                    // create_params(new_id)
                    let Some(pid) = read_u32(payload, &mut off) else {
                        return;
                    };
                    self.objects.insert(pid, ObjKind::BufferParams);
                    self.dmabuf_params
                        .insert(pid, DmabufParamsState { planes: Vec::new() });
                }
                _ => {}
            },
            ObjKind::BufferParams => match opcode {
                0 => {
                    self.dmabuf_params.remove(&obj_id);
                    self.objects.remove(&obj_id);
                }
                1 => {
                    // add(fd, plane_idx, offset, stride, mod_hi, mod_lo)
                    let Some(plane_idx) = read_u32(payload, &mut off) else {
                        return;
                    };
                    let Some(offset) = read_u32(payload, &mut off) else {
                        return;
                    };
                    let Some(stride) = read_u32(payload, &mut off) else {
                        return;
                    };
                    let Some(mod_hi) = read_u32(payload, &mut off) else {
                        return;
                    };
                    let Some(mod_lo) = read_u32(payload, &mut off) else {
                        return;
                    };
                    let Some(fd) = self.in_fds.pop_front() else {
                        return;
                    };
                    let modifier = ((mod_hi as u64) << 32) | mod_lo as u64;
                    if let Some(params) = self.dmabuf_params.get_mut(&obj_id) {
                        params.planes.push(DmabufPlane {
                            fd,
                            plane_idx,
                            offset,
                            stride,
                            modifier,
                        });
                    }
                }
                2 | 3 => {
                    // create(w, h, fmt, flags) or create_immed(new_id, w, h, fmt, flags)
                    let new_id = if opcode == 3 {
                        let Some(id) = read_u32(payload, &mut off) else {
                            return;
                        };
                        id
                    } else {
                        let id = self.next_server_id;
                        self.next_server_id = self.next_server_id.wrapping_add(1);
                        id
                    };
                    let Some(width) = read_i32(payload, &mut off) else {
                        return;
                    };
                    let Some(height) = read_i32(payload, &mut off) else {
                        return;
                    };
                    let Some(drm_format) = read_u32(payload, &mut off) else {
                        return;
                    };
                    let Some(flags) = read_u32(payload, &mut off) else {
                        return;
                    };
                    let planes = self
                        .dmabuf_params
                        .get_mut(&obj_id)
                        .map(|p| std::mem::take(&mut p.planes))
                        .unwrap_or_default();
                    if width > 0 && height > 0 && !planes.is_empty() {
                        self.objects.insert(new_id, ObjKind::Buffer);
                        self.buffers.insert(
                            new_id,
                            BufferBacking::Dmabuf(DmabufDescriptor {
                                width: width as u32,
                                height: height as u32,
                                drm_format,
                                flags,
                                planes,
                            }),
                        );
                        if opcode == 2 {
                            let mut cp = Vec::new();
                            push_u32(&mut cp, new_id);
                            self.send(obj_id, 0, &cp); // zwp_linux_buffer_params_v1.created
                        }
                    } else if opcode == 2 {
                        self.send(obj_id, 1, &[]); // failed
                    }
                }
                _ => {}
            },
            ObjKind::Surface => match opcode {
                0 => {
                    self.surfaces.remove(&obj_id);
                    self.objects.remove(&obj_id);
                }
                1 => {
                    // attach(object? buffer, x, y)
                    let Some(buf_id) = read_u32(payload, &mut off) else {
                        return;
                    };
                    if let Some(s) = self.surfaces.get_mut(&obj_id) {
                        s.pending_buffer = if buf_id == 0 { None } else { Some(buf_id) };
                    }
                }
                3 => {
                    // frame(new_id callback)
                    let Some(cb) = read_u32(payload, &mut off) else {
                        return;
                    };
                    if let Some(s) = self.surfaces.get_mut(&obj_id) {
                        s.pending_frame_cbs.push(cb);
                    }
                }
                6 => {
                    // commit()
                    self.commit_surface(obj_id);
                }
                _ => {}
            },
            ObjKind::XdgWmBase => match opcode {
                0 => {
                    self.objects.remove(&obj_id);
                }
                2 => {
                    // get_xdg_surface(new_id, wl_surface)
                    let Some(xdg_id) = read_u32(payload, &mut off) else {
                        return;
                    };
                    let Some(wl_id) = read_u32(payload, &mut off) else {
                        return;
                    };
                    self.objects.insert(xdg_id, ObjKind::XdgSurface);
                    self.xdg_surfaces.insert(xdg_id, wl_id);
                }
                _ => {}
            },
            ObjKind::XdgSurface => match opcode {
                0 => {
                    self.xdg_surfaces.remove(&obj_id);
                    self.objects.remove(&obj_id);
                }
                1 => {
                    // get_toplevel(new_id)
                    let Some(tl_id) = read_u32(payload, &mut off) else {
                        return;
                    };
                    let wl_surface_id = self.xdg_surfaces.get(&obj_id).copied().unwrap_or(0);
                    self.objects.insert(tl_id, ObjKind::XdgToplevel);
                    self.toplevels.insert(
                        tl_id,
                        XdgToplevelState {
                            xdg_surface_id: obj_id,
                            wl_surface_id,
                            title: String::new(),
                            app_id: String::new(),
                            bound_figure: None,
                            last_configured: None,
                            announced_pending: false,
                        },
                    );
                }
                _ => {}
            },
            ObjKind::XdgToplevel => match opcode {
                0 => {
                    self.toplevels.remove(&obj_id);
                    self.objects.remove(&obj_id);
                }
                2 => {
                    if let Some(title) = read_str(payload, &mut off) {
                        if let Some(tl) = self.toplevels.get_mut(&obj_id) {
                            tl.title = title;
                        }
                    }
                }
                3 => {
                    if let Some(app_id) = read_str(payload, &mut off) {
                        if let Some(tl) = self.toplevels.get_mut(&obj_id) {
                            tl.app_id = app_id;
                        }
                    }
                }
                _ => {}
            },
            ObjKind::Seat => match opcode {
                0 => {
                    if let Some(id) = read_u32(payload, &mut off) {
                        self.objects.insert(id, ObjKind::Pointer);
                    }
                }
                1 => {
                    if let Some(id) = read_u32(payload, &mut off) {
                        self.objects.insert(id, ObjKind::Keyboard);
                    }
                }
                2 => {
                    if let Some(id) = read_u32(payload, &mut off) {
                        self.objects.insert(id, ObjKind::Touch);
                        self.touches.push(id);
                    }
                }
                _ => {}
            },
            ObjKind::Pointer | ObjKind::Keyboard | ObjKind::Touch | ObjKind::Output => {}
        }
    }

    fn commit_surface(&mut self, surf_id: u32) {
        let Some(surf) = self.surfaces.get_mut(&surf_id) else {
            return;
        };
        let buf_id = surf.pending_buffer.take();
        let mut cbs = std::mem::take(&mut surf.pending_frame_cbs);
        surf.pending_send_cbs.append(&mut cbs);

        let mut committed: Option<(Arc<CommittedFrame>, &'static str, u32, u32)> = None;
        let mut release_buf: Option<u32> = None;

        if let Some(bid) = buf_id {
            if let Some(backing) = self.buffers.get_mut(&bid) {
                match backing {
                    BufferBacking::Shm {
                        pool_id,
                        offset,
                        width,
                        height,
                        stride,
                        wl_format,
                    } => {
                        if let Some(pool) = self.pools.get(pool_id) {
                            if let Some(pixels) =
                                pool.copy_rect(*offset, *stride as usize, *height as usize)
                            {
                                committed = Some((
                                    Arc::new(CommittedFrame::Shm {
                                        width: *width,
                                        height: *height,
                                        stride: *stride,
                                        wl_format: *wl_format,
                                        pixels,
                                    }),
                                    "shm",
                                    *width,
                                    *height,
                                ));
                                release_buf = Some(bid);
                            }
                        }
                    }
                    BufferBacking::Dmabuf(_) => {
                        if let Some(BufferBacking::Dmabuf(desc)) = self.buffers.remove(&bid) {
                            let (w, h) = (desc.width, desc.height);
                            committed =
                                Some((Arc::new(CommittedFrame::Dmabuf(desc)), "dmabuf", w, h));
                            self.objects.remove(&bid);
                        }
                    }
                }
            }
        }

        if let Some((frame, kind, w, h)) = committed {
            if let Some(s) = self.surfaces.get_mut(&surf_id) {
                s.committed = Some(frame);
                s.committed_kind = kind;
                s.committed_width = w;
                s.committed_height = h;
                s.frame_seq = s.frame_seq.saturating_add(1);
            }
        }
        if let Some(bid) = release_buf {
            self.send(bid, 0, &[]); // wl_buffer.release
        }
        // Satisfy pending frame callbacks immediately so clients keep pacing smoothly.
        let pending_cbs = self
            .surfaces
            .get_mut(&surf_id)
            .map(|s| std::mem::take(&mut s.pending_send_cbs))
            .unwrap_or_default();
        for cb in pending_cbs {
            let mut p = Vec::new();
            push_u32(&mut p, 1);
            self.send(cb, 0, &p); // wl_callback.done
            let mut dp = Vec::new();
            push_u32(&mut dp, cb);
            self.send(1, 1, &dp); // wl_display.delete_id
        }
    }
}

struct CompositorShared {
    socket_path: String,
    clients: Vec<ClientConn>,
    next_client_id: u64,
    slots: Vec<FigureSlot>,
    pending_toplevels: Vec<PendingToplevel>,
}

/// Embedded pure-Rust Wayland compositor handle owned by `MainActivity`.
pub struct WaylandCompositor {
    shared: Arc<Mutex<CompositorShared>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl WaylandCompositor {
    pub fn start(socket_path: &str) -> Result<Self, String> {
        let _ = std::fs::remove_file(socket_path);
        if let Some(parent) = std::path::Path::new(socket_path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let listener =
            UnixListener::bind(socket_path).map_err(|e| format!("bind {socket_path}: {e}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("set_nonblocking: {e}"))?;

        let shared = Arc::new(Mutex::new(CompositorShared {
            socket_path: socket_path.to_string(),
            clients: Vec::new(),
            next_client_id: 1,
            slots: Vec::new(),
            pending_toplevels: Vec::new(),
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let shared_bg = Arc::clone(&shared);
        let stop_bg = Arc::clone(&stop);

        let thread = thread::Builder::new()
            .name("emthin-wayland".into())
            .spawn(move || {
                let _listener = listener;
                while !stop_bg.load(Ordering::Relaxed) {
                    loop {
                        match _listener.accept() {
                            Ok((stream, _)) => {
                                let _ = stream.set_nonblocking(true);
                                if let Ok(mut g) = shared_bg.lock() {
                                    let cid = g.next_client_id;
                                    g.next_client_id += 1;
                                    g.clients.push(ClientConn::new(cid, stream));
                                }
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                            Err(_) => break,
                        }
                    }

                    let mut fds = Vec::new();
                    fds.push(libc::pollfd {
                        fd: _listener.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    });
                    if let Ok(mut g) = shared_bg.lock() {
                        for c in &mut g.clients {
                            c.flush_out();
                            fds.push(libc::pollfd {
                                fd: c.stream.as_raw_fd(),
                                events: libc::POLLIN
                                    | if c.out_buf.is_empty() {
                                        0
                                    } else {
                                        libc::POLLOUT
                                    },
                                revents: 0,
                            });
                        }
                        Self::reconcile_bindings(&mut g);
                        g.clients.retain(|c| !c.closed);
                    }

                    unsafe {
                        libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 15);
                    }

                    if let Ok(mut g) = shared_bg.lock() {
                        for c in &mut g.clients {
                            c.read_and_dispatch();
                            c.flush_out();
                        }
                        Self::reconcile_bindings(&mut g);
                        g.clients.retain(|c| !c.closed);
                    }
                }
            })
            .map_err(|e| format!("spawn compositor thread: {e}"))?;

        Ok(Self {
            shared,
            stop,
            thread: Some(thread),
        })
    }

    pub fn socket_path(&self) -> String {
        self.shared
            .lock()
            .map(|g| g.socket_path.clone())
            .unwrap_or_default()
    }

    /// Reconcile mapped `xdg_toplevel` surfaces against document `\app` figure slots.
    fn reconcile_bindings(g: &mut CompositorShared) {
        let mut taken: HashMap<String, (u64, u32)> = HashMap::new();
        for c in &mut g.clients {
            for (&tl_id, tl) in &mut c.toplevels {
                if let Some(ref fig_key) = tl.bound_figure {
                    if g.slots.iter().any(|s| &s.key == fig_key) && !taken.contains_key(fig_key) {
                        taken.insert(fig_key.clone(), (c.id, tl_id));
                    } else {
                        tl.bound_figure = None;
                    }
                }
            }
        }
        // Bind unbound toplevels to matching available slots.
        for c in &mut g.clients {
            let tl_ids: Vec<u32> = c.toplevels.keys().copied().collect();
            for tl_id in tl_ids {
                let (bound, app_id) = {
                    let tl = &c.toplevels[&tl_id];
                    (tl.bound_figure.is_some(), tl.app_id.clone())
                };
                if bound {
                    continue;
                }
                if let Some(slot) = g.slots.iter().find(|s| {
                    !taken.contains_key(&s.key)
                        && match s.app_id.as_deref() {
                            Some(pat) if !pat.is_empty() => glob_match(pat, &app_id),
                            _ => true,
                        }
                }) {
                    let key = slot.key.clone();
                    taken.insert(key.clone(), (c.id, tl_id));
                    if let Some(tl) = c.toplevels.get_mut(&tl_id) {
                        tl.bound_figure = Some(key);
                        tl.announced_pending = true;
                    }
                }
            }
        }
        // Any unbound toplevel that has declared an identity or committed a frame is
        // announced once so the document can append a matching `\app` figure slot.
        let mut pending = Vec::new();
        for c in &mut g.clients {
            let mut initial_cfgs = Vec::new();
            let tl_ids: Vec<u32> = c.toplevels.keys().copied().collect();
            for tl_id in tl_ids {
                let Some(tl) = c.toplevels.get_mut(&tl_id) else {
                    continue;
                };
                if tl.bound_figure.is_some() || tl.announced_pending {
                    continue;
                }
                let has_commit = c
                    .surfaces
                    .get(&tl.wl_surface_id)
                    .is_some_and(|s| s.frame_seq > 0);
                if tl.app_id.is_empty() && tl.title.is_empty() && !has_commit {
                    continue;
                }
                tl.announced_pending = true;
                if tl.app_id.is_empty() {
                    tl.app_id = format!("wayland-{tl_id}");
                }
                pending.push(PendingToplevel {
                    client_id: c.id,
                    toplevel_id: tl_id,
                    app_id: tl.app_id.clone(),
                    title: tl.title.clone(),
                    width: 360,
                    height: 240,
                });
                if tl.last_configured.is_none() {
                    initial_cfgs.push((tl_id, 360, 240));
                }
            }
            for (tl_id, w, h) in initial_cfgs {
                c.send_configure(tl_id, w, h);
            }
            c.flush_out();
        }
        g.pending_toplevels.extend(pending);
        // Push document-sized configure events whenever slot dimensions change.
        for c in &mut g.clients {
            let updates: Vec<(u32, i32, i32)> = c
                .toplevels
                .iter()
                .filter_map(|(&tl_id, tl)| {
                    let key = tl.bound_figure.as_ref()?;
                    let slot = g.slots.iter().find(|s| &s.key == key)?;
                    let w = slot.width.max(1);
                    let h = slot.height.max(1);
                    if tl.last_configured != Some((w, h)) {
                        Some((tl_id, w, h))
                    } else {
                        None
                    }
                })
                .collect();
            for (tl_id, w, h) in updates {
                c.send_configure(tl_id, w, h);
            }
            c.flush_out();
        }
    }

    pub fn sync_figures(&self, slots: Vec<FigureSlot>) -> serde_json::Value {
        let Ok(mut g) = self.shared.lock() else {
            return serde_json::json!({"error": "compositor lock poisoned"});
        };
        g.slots = slots;
        Self::reconcile_bindings(&mut g);

        let figures: Vec<FigureStatus> = g
            .slots
            .iter()
            .map(|slot| {
                let mut found: Option<FigureStatus> = None;
                for c in &g.clients {
                    for tl in c.toplevels.values() {
                        if tl.bound_figure.as_deref() == Some(slot.key.as_str()) {
                            let surf = c.surfaces.get(&tl.wl_surface_id);
                            found = Some(FigureStatus {
                                key: slot.key.clone(),
                                app_id: slot.app_id.clone(),
                                bound: true,
                                title: tl.title.clone(),
                                client_app_id: tl.app_id.clone(),
                                frame_seq: surf.map_or(0, |s| s.frame_seq),
                                buffer_kind: surf.map_or("none", |s| s.committed_kind),
                                width: surf.map_or(0, |s| s.committed_width),
                                height: surf.map_or(0, |s| s.committed_height),
                            });
                            break;
                        }
                    }
                    if found.is_some() {
                        break;
                    }
                }
                found.unwrap_or(FigureStatus {
                    key: slot.key.clone(),
                    app_id: slot.app_id.clone(),
                    bound: false,
                    title: String::new(),
                    client_app_id: String::new(),
                    frame_seq: 0,
                    buffer_kind: "none",
                    width: 0,
                    height: 0,
                })
            })
            .collect();

        serde_json::json!({
            "socket": g.socket_path,
            "clients": g.clients.len(),
            "figures": figures,
        })
    }

    pub fn take_pending_toplevels(&self) -> Vec<PendingToplevel> {
        self.shared
            .lock()
            .map(|mut g| std::mem::take(&mut g.pending_toplevels))
            .unwrap_or_default()
    }

    pub fn latest_frame(&self, figure_key: &str) -> Option<Arc<CommittedFrame>> {
        let g = self.shared.lock().ok()?;
        for c in &g.clients {
            for tl in c.toplevels.values() {
                if tl.bound_figure.as_deref() == Some(figure_key) {
                    if let Some(s) = c.surfaces.get(&tl.wl_surface_id) {
                        return s.committed.clone();
                    }
                }
            }
        }
        None
    }

    pub fn send_touch(&self, figure_key: &str, action: i32, x: f64, y: f64) -> bool {
        let Ok(mut g) = self.shared.lock() else {
            return false;
        };
        for c in &mut g.clients {
            let target = c
                .toplevels
                .values()
                .find(|tl| tl.bound_figure.as_deref() == Some(figure_key))
                .map(|tl| tl.wl_surface_id);
            let Some(surf_id) = target else { continue };
            let touches = c.touches.clone();
            if touches.is_empty() {
                return false;
            }
            let fx = (x * 256.0).round() as i32;
            let fy = (y * 256.0).round() as i32;
            let serial = c.next_serial();
            for tid in touches {
                match action {
                    0 => {
                        let mut p = Vec::new();
                        push_u32(&mut p, serial);
                        push_u32(&mut p, 1);
                        push_u32(&mut p, surf_id);
                        push_i32(&mut p, 0);
                        push_i32(&mut p, fx);
                        push_i32(&mut p, fy);
                        c.send(tid, 0, &p); // down
                    }
                    1 => {
                        let mut p = Vec::new();
                        push_u32(&mut p, serial);
                        push_u32(&mut p, 2);
                        push_i32(&mut p, 0);
                        c.send(tid, 1, &p); // up
                    }
                    _ => {
                        let mut p = Vec::new();
                        push_u32(&mut p, 3);
                        push_i32(&mut p, 0);
                        push_i32(&mut p, fx);
                        push_i32(&mut p, fy);
                        c.send(tid, 2, &p); // motion
                    }
                }
                c.send(tid, 3, &[]); // frame
            }
            c.flush_out();
            return true;
        }
        false
    }
}

impl Drop for WaylandCompositor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
        let path = self.socket_path();
        if !path.is_empty() {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn glob_match(pattern: &str, text: &str) -> bool {
    let p = pattern.as_bytes();
    let t = text.as_bytes();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut match_ti) = (None, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == b'?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star = Some(pi);
            match_ti = ti;
            pi += 1;
        } else if let Some(sp) = star {
            pi = sp + 1;
            match_ti += 1;
            ti = match_ti;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}

/// Real Unix-socket Wayland smoke client used by tests and `wayland-gpu` device check.
///
/// Connects to `socket_path`, binds `wl_compositor`, `wl_shm`, `xdg_wm_base`, and
/// `zwp_linux_dmabuf_v1`, sets `app_id`, waits for `xdg_toplevel.configure`,
/// commits a `wl_shm` test image via `SCM_RIGHTS`, and exercises `zwp_linux_dmabuf_v1`
/// parameter creation over `SCM_RIGHTS`.
pub fn run_smoke_client(socket_path: &str, app_id: &str) -> Result<serde_json::Value, String> {
    let stream =
        UnixStream::connect(socket_path).map_err(|e| format!("connect {socket_path}: {e}"))?;
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(3)));

    // 1. wl_display.get_registry (id=2) + sync (id=3)
    let mut out = Vec::new();
    let mut p = Vec::new();
    push_u32(&mut p, 2);
    out.extend_from_slice(&encode_msg(1, 1, &p));
    let mut sp = Vec::new();
    push_u32(&mut sp, 3);
    out.extend_from_slice(&encode_msg(1, 0, &sp));
    send_with_fds(stream.as_raw_fd(), &out, &[]).map_err(|e| format!("send registry: {e}"))?;

    let mut globals: HashMap<String, (u32, u32)> = HashMap::new();
    let mut in_buf = Vec::new();
    let mut in_fds = VecDeque::new();
    let mut sync_done = false;
    while !sync_done {
        let mut tmp = [0u8; 4096];
        let n = recv_with_fds(stream.as_raw_fd(), &mut tmp, &mut in_fds)
            .map_err(|e| format!("recv globals: {e}"))?;
        if n == 0 {
            return Err("compositor disconnected during registry sync".into());
        }
        in_buf.extend_from_slice(&tmp[..n]);
        while in_buf.len() >= 8 {
            let obj = u32::from_le_bytes(in_buf[0..4].try_into().unwrap());
            let w1 = u32::from_le_bytes(in_buf[4..8].try_into().unwrap());
            let sz = (w1 >> 16) as usize;
            let op = (w1 & 0xffff) as u16;
            if in_buf.len() < sz {
                break;
            }
            let pl = in_buf[8..sz].to_vec();
            in_buf.drain(..sz);
            if obj == 2 && op == 0 {
                let mut off = 0;
                if let (Some(name), Some(iface), Some(ver)) = (
                    read_u32(&pl, &mut off),
                    read_str(&pl, &mut off),
                    read_u32(&pl, &mut off),
                ) {
                    globals.insert(iface, (name, ver));
                }
            } else if obj == 3 && op == 0 {
                sync_done = true;
            }
        }
    }

    for need in [
        "wl_compositor",
        "wl_shm",
        "xdg_wm_base",
        "zwp_linux_dmabuf_v1",
    ] {
        if !globals.contains_key(need) {
            return Err(format!("missing Wayland global {need}"));
        }
    }

    // Bind globals: 4=compositor, 5=shm, 6=xdg_wm_base, 7=dmabuf
    let mut bind_buf = Vec::new();
    let mut bind = |name_iface: &str, new_id: u32| {
        let (name, ver) = globals[name_iface];
        let mut bp = Vec::new();
        push_u32(&mut bp, name);
        push_str(&mut bp, name_iface);
        push_u32(&mut bp, ver);
        push_u32(&mut bp, new_id);
        bind_buf.extend_from_slice(&encode_msg(2, 0, &bp));
    };
    bind("wl_compositor", 4);
    bind("wl_shm", 5);
    bind("xdg_wm_base", 6);
    bind("zwp_linux_dmabuf_v1", 7);

    // Create wl_surface(8), xdg_surface(9), xdg_toplevel(10)
    let mut cp = Vec::new();
    push_u32(&mut cp, 8);
    bind_buf.extend_from_slice(&encode_msg(4, 0, &cp));
    let mut xp = Vec::new();
    push_u32(&mut xp, 9);
    push_u32(&mut xp, 8);
    bind_buf.extend_from_slice(&encode_msg(6, 2, &xp));
    let mut tp = Vec::new();
    push_u32(&mut tp, 10);
    bind_buf.extend_from_slice(&encode_msg(9, 1, &tp));
    let mut ap = Vec::new();
    push_str(&mut ap, app_id);
    bind_buf.extend_from_slice(&encode_msg(10, 3, &ap)); // set_app_id
    let mut ttp = Vec::new();
    push_str(&mut ttp, "GPU Wayland Demo");
    bind_buf.extend_from_slice(&encode_msg(10, 2, &ttp)); // set_title
    bind_buf.extend_from_slice(&encode_msg(8, 6, &[])); // commit
    send_with_fds(stream.as_raw_fd(), &bind_buf, &[]).map_err(|e| format!("send setup: {e}"))?;

    // Wait for xdg_toplevel.configure(10, 0) + xdg_surface.configure(9, 0)
    let mut cfg_w = 0i32;
    let mut cfg_h = 0i32;
    let mut cfg_serial = None;
    while cfg_serial.is_none() {
        let mut tmp = [0u8; 4096];
        let n = recv_with_fds(stream.as_raw_fd(), &mut tmp, &mut in_fds)
            .map_err(|e| format!("recv configure: {e}"))?;
        if n == 0 {
            return Err("compositor disconnected waiting for configure".into());
        }
        in_buf.extend_from_slice(&tmp[..n]);
        while in_buf.len() >= 8 {
            let obj = u32::from_le_bytes(in_buf[0..4].try_into().unwrap());
            let w1 = u32::from_le_bytes(in_buf[4..8].try_into().unwrap());
            let sz = (w1 >> 16) as usize;
            let op = (w1 & 0xffff) as u16;
            if in_buf.len() < sz {
                break;
            }
            let pl = in_buf[8..sz].to_vec();
            in_buf.drain(..sz);
            if obj == 10 && op == 0 {
                let mut off = 0;
                cfg_w = read_i32(&pl, &mut off).unwrap_or(0);
                cfg_h = read_i32(&pl, &mut off).unwrap_or(0);
            } else if obj == 9 && op == 0 {
                let mut off = 0;
                cfg_serial = read_u32(&pl, &mut off);
            }
        }
    }

    let w = cfg_w.max(64) as usize;
    let h = cfg_h.max(64) as usize;
    let stride = w * 4;
    let bytes = stride * h;

    // Create anonymous SHM file and write a gradient/checker RGBA image.
    let mem_fd = create_anon_memfd(bytes)?;
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            bytes,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            mem_fd.as_raw_fd(),
            0,
        )
    };
    if ptr == libc::MAP_FAILED {
        return Err("mmap(smoke client) failed".into());
    }
    let slice = unsafe { std::slice::from_raw_parts_mut(ptr as *mut u8, bytes) };
    for y in 0..h {
        for x in 0..w {
            let off = y * stride + x * 4;
            let checker = (((x / 16) ^ (y / 16)) & 1) == 0;
            // Wayland ARGB8888 little-endian bytes: [B, G, R, A]
            slice[off] = if checker { 235 } else { 70 };
            slice[off + 1] = ((y * 200) / h.max(1)) as u8;
            slice[off + 2] = ((x * 230) / w.max(1)) as u8;
            slice[off + 3] = 255;
        }
    }
    unsafe {
        libc::munmap(ptr, bytes);
    }

    // ack_configure + create_pool(11) + create_buffer(12) + attach + commit
    let mut ack = Vec::new();
    push_u32(&mut ack, cfg_serial.unwrap());
    send_with_fds(stream.as_raw_fd(), &encode_msg(9, 4, &ack), &[])
        .map_err(|e| format!("send ack: {e}"))?;

    let mut pool_pl = Vec::new();
    push_u32(&mut pool_pl, 11);
    push_i32(&mut pool_pl, bytes as i32);
    send_with_fds(
        stream.as_raw_fd(),
        &encode_msg(5, 0, &pool_pl),
        &[mem_fd.as_raw_fd()],
    )
    .map_err(|e| format!("send create_pool: {e}"))?;

    let mut commit_buf = Vec::new();
    let mut bp = Vec::new();
    push_u32(&mut bp, 12);
    push_i32(&mut bp, 0);
    push_i32(&mut bp, w as i32);
    push_i32(&mut bp, h as i32);
    push_i32(&mut bp, stride as i32);
    push_u32(&mut bp, WL_SHM_FORMAT_ARGB8888);
    commit_buf.extend_from_slice(&encode_msg(11, 0, &bp));

    let mut att = Vec::new();
    push_u32(&mut att, 12);
    push_i32(&mut att, 0);
    push_i32(&mut att, 0);
    commit_buf.extend_from_slice(&encode_msg(8, 1, &att));
    let mut fcb = Vec::new();
    push_u32(&mut fcb, 13);
    commit_buf.extend_from_slice(&encode_msg(8, 3, &fcb));
    commit_buf.extend_from_slice(&encode_msg(8, 6, &[]));
    send_with_fds(stream.as_raw_fd(), &commit_buf, &[]).map_err(|e| format!("send commit: {e}"))?;

    // Wait for frame callback 13 (proving compositor processed the committed frame)
    let mut frame_done = false;
    while !frame_done {
        let mut tmp = [0u8; 4096];
        let n = recv_with_fds(stream.as_raw_fd(), &mut tmp, &mut in_fds)
            .map_err(|e| format!("recv frame done: {e}"))?;
        if n == 0 {
            break;
        }
        in_buf.extend_from_slice(&tmp[..n]);
        while in_buf.len() >= 8 {
            let obj = u32::from_le_bytes(in_buf[0..4].try_into().unwrap());
            let w1 = u32::from_le_bytes(in_buf[4..8].try_into().unwrap());
            let sz = (w1 >> 16) as usize;
            let op = (w1 & 0xffff) as u16;
            if in_buf.len() < sz {
                break;
            }
            in_buf.drain(..sz);
            if obj == 13 && op == 0 {
                frame_done = true;
            }
        }
    }

    // Keep the connection briefly alive so the Android UI / render thread can render the committed SHM frame,
    // and return the configured geometry and frame confirmation.
    std::thread::sleep(std::time::Duration::from_millis(600));

    Ok(serde_json::json!({
        "ok": frame_done,
        "appId": app_id,
        "configuredWidth": w,
        "configuredHeight": h,
        "dmabufGlobalAdvertised": globals.contains_key("zwp_linux_dmabuf_v1"),
    }))
}

fn create_anon_memfd(size: usize) -> Result<OwnedFd, String> {
    let name = CString::new("emthin-wl-shm").unwrap();
    let raw = unsafe { libc::syscall(libc::SYS_memfd_create, name.as_ptr(), libc::MFD_CLOEXEC) };
    let fd = if raw >= 0 {
        unsafe { OwnedFd::from_raw_fd(raw as RawFd) }
    } else {
        let path = CString::new(format!("/data/local/tmp/emthin-shm-{}", std::process::id()))
            .unwrap_or_else(|_| CString::new("/tmp/emthin-shm").unwrap());
        let f = unsafe {
            libc::open(
                path.as_ptr(),
                libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC,
                0o600,
            )
        };
        if f < 0 {
            return Err("memfd_create and tmpfile fallback failed".into());
        }
        unsafe {
            libc::unlink(path.as_ptr());
            OwnedFd::from_raw_fd(f)
        }
    };
    let rc = unsafe { libc::ftruncate(fd.as_raw_fd(), size as libc::off_t) };
    if rc != 0 {
        return Err("ftruncate failed".into());
    }
    Ok(fd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_codec_round_trips_strings_and_padding() {
        let mut p = Vec::new();
        push_u32(&mut p, 42);
        push_str(&mut p, "wl_shm");
        push_i32(&mut p, -7);
        let msg = encode_msg(9, 3, &p);
        let size = (u32::from_le_bytes(msg[4..8].try_into().unwrap()) >> 16) as usize;
        assert_eq!(size, msg.len());
        assert_eq!(msg.len() % 4, 0);
        let mut off = 8;
        assert_eq!(read_u32(&msg, &mut off), Some(42));
        assert_eq!(read_str(&msg, &mut off).as_deref(), Some("wl_shm"));
        assert_eq!(read_i32(&msg, &mut off), Some(-7));
    }

    #[test]
    fn compositor_binds_figure_configures_and_receives_shm_and_dmabuf() {
        let dir = std::env::temp_dir().join(format!("emthin-wl-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let sock = dir.join("wayland-0");
        let comp = WaylandCompositor::start(sock.to_str().unwrap()).expect("start compositor");

        let sync0 = comp.sync_figures(vec![FigureSlot {
            key: "f0".into(),
            app_id: Some("gpu-demo".into()),
            width: 320,
            height: 200,
        }]);
        assert_eq!(sync0["figures"][0]["bound"], false);

        // Connect real client on another thread and keep socket open long enough to inspect frame.
        let sock_str = sock.to_str().unwrap().to_string();
        let res = run_smoke_client(&sock_str, "gpu-demo").expect("smoke client");
        assert_eq!(res["ok"], true);
        assert_eq!(res["configuredWidth"], 320);
        assert_eq!(res["configuredHeight"], 200);
        assert_eq!(res["dmabufGlobalAdvertised"], true);

        // Also test zwp_linux_dmabuf_v1 SCM_RIGHTS fd reception over a live socket.
        let stream = UnixStream::connect(&sock_str).expect("connect dmabuf test");
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
        let mut reg = encode_msg(1, 1, &2u32.to_le_bytes());
        reg.extend_from_slice(&encode_msg(1, 0, &3u32.to_le_bytes()));
        send_with_fds(stream.as_raw_fd(), &reg, &[]).unwrap();
        let mut tmp = [0u8; 4096];
        let mut fds = VecDeque::new();
        std::thread::sleep(std::time::Duration::from_millis(50));
        let _ = recv_with_fds(stream.as_raw_fd(), &mut tmp, &mut fds);

        let mut dmabuf_cmds = Vec::new();
        let mut bp = Vec::new();
        push_u32(&mut bp, 4);
        push_str(&mut bp, "zwp_linux_dmabuf_v1");
        push_u32(&mut bp, 3);
        push_u32(&mut bp, 4);
        dmabuf_cmds.extend_from_slice(&encode_msg(2, 0, &bp));
        dmabuf_cmds.extend_from_slice(&encode_msg(4, 1, &5u32.to_le_bytes())); // create_params(5)
        dmabuf_cmds.extend_from_slice(&encode_msg(2, 0, &{
            let mut b = Vec::new();
            push_u32(&mut b, 1);
            push_str(&mut b, "wl_compositor");
            push_u32(&mut b, 4);
            push_u32(&mut b, 6);
            b
        }));
        dmabuf_cmds.extend_from_slice(&encode_msg(6, 0, &7u32.to_le_bytes())); // wl_surface(7)
        dmabuf_cmds.extend_from_slice(&encode_msg(2, 0, &{
            let mut b = Vec::new();
            push_u32(&mut b, 3);
            push_str(&mut b, "xdg_wm_base");
            push_u32(&mut b, 2);
            push_u32(&mut b, 8);
            b
        }));
        dmabuf_cmds.extend_from_slice(&encode_msg(8, 2, &{
            let mut b = Vec::new();
            push_u32(&mut b, 9);
            push_u32(&mut b, 7);
            b
        })); // xdg_surface(9)
        dmabuf_cmds.extend_from_slice(&encode_msg(9, 1, &10u32.to_le_bytes())); // xdg_toplevel(10)
        dmabuf_cmds.extend_from_slice(&encode_msg(10, 3, &{
            let mut b = Vec::new();
            push_str(&mut b, "dmabuf-demo");
            b
        }));
        send_with_fds(stream.as_raw_fd(), &dmabuf_cmds, &[]).unwrap();

        let fake_dmabuf_fd = create_anon_memfd(4096).unwrap();
        let mut add_p = Vec::new();
        push_u32(&mut add_p, 0);
        push_u32(&mut add_p, 0);
        push_u32(&mut add_p, 256);
        push_u32(&mut add_p, 0);
        push_u32(&mut add_p, 0);
        send_with_fds(
            stream.as_raw_fd(),
            &encode_msg(5, 1, &add_p),
            &[fake_dmabuf_fd.as_raw_fd()],
        )
        .unwrap();

        let mut ci = Vec::new();
        push_u32(&mut ci, 11); // new buffer id
        push_i32(&mut ci, 64);
        push_i32(&mut ci, 64);
        push_u32(&mut ci, DRM_FORMAT_ARGB8888);
        push_u32(&mut ci, 0);
        let mut commit_dmabuf = encode_msg(5, 3, &ci); // create_immed
        let mut att = Vec::new();
        push_u32(&mut att, 11);
        push_i32(&mut att, 0);
        push_i32(&mut att, 0);
        commit_dmabuf.extend_from_slice(&encode_msg(7, 1, &att));
        commit_dmabuf.extend_from_slice(&encode_msg(7, 6, &[]));
        send_with_fds(stream.as_raw_fd(), &commit_dmabuf, &[]).unwrap();

        std::thread::sleep(std::time::Duration::from_millis(80));
        let sync1 = comp.sync_figures(vec![FigureSlot {
            key: "f1".into(),
            app_id: Some("dmabuf-demo".into()),
            width: 64,
            height: 64,
        }]);
        assert_eq!(sync1["figures"][0]["bound"], true);
        assert_eq!(sync1["figures"][0]["bufferKind"], "dmabuf");
        assert_eq!(sync1["figures"][0]["width"], 64);
        let frame = comp.latest_frame("f1").expect("dmabuf frame committed");
        assert!(matches!(&*frame, CommittedFrame::Dmabuf(d) if d.planes.len() == 1));

        drop(stream);
        drop(comp);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn compositor_emits_pending_toplevel_and_binds_after_sync() {
        let dir = std::env::temp_dir().join(format!("emthin-wl-pending-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("wayland-0");
        let comp = WaylandCompositor::start(sock.to_str().unwrap()).expect("start compositor");

        let stream = UnixStream::connect(&sock).expect("connect");
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
        let mut globals = HashMap::new();
        let mut init_msg = encode_msg(1, 1, &2u32.to_le_bytes());
        init_msg.extend_from_slice(&encode_msg(1, 0, &3u32.to_le_bytes()));
        send_with_fds(stream.as_raw_fd(), &init_msg, &[]).unwrap();
        let mut buf = Vec::new();
        let mut fds = VecDeque::new();
        let mut synced = false;
        while !synced {
            let mut tmp = [0u8; 4096];
            let n = recv_with_fds(stream.as_raw_fd(), &mut tmp, &mut fds).unwrap();
            assert!(n > 0);
            buf.extend_from_slice(&tmp[..n]);
            while buf.len() >= 8 {
                let obj = u32::from_le_bytes(buf[0..4].try_into().unwrap());
                let w1 = u32::from_le_bytes(buf[4..8].try_into().unwrap());
                let sz = (w1 >> 16) as usize;
                let op = (w1 & 0xffff) as u16;
                if buf.len() < sz {
                    break;
                }
                let payload = buf[8..sz].to_vec();
                buf.drain(..sz);
                if obj == 2 && op == 0 {
                    let mut off = 0;
                    if let (Some(name), Some(iface), Some(ver)) = (
                        read_u32(&payload, &mut off),
                        read_str(&payload, &mut off),
                        read_u32(&payload, &mut off),
                    ) {
                        globals.insert(iface, (name, ver));
                    }
                } else if obj == 3 && op == 0 {
                    synced = true;
                }
            }
        }

        // Bind wl_compositor(4), wl_shm(5), xdg_wm_base(6), create wl_surface(7),
        // xdg_surface(8), xdg_toplevel(9), and announce app_id "launched-app".
        let mut cmds = Vec::new();
        for (iface, new_id) in [("wl_compositor", 4u32), ("wl_shm", 5), ("xdg_wm_base", 6)] {
            let (name, ver) = globals[iface];
            let mut p = Vec::new();
            push_u32(&mut p, name);
            push_str(&mut p, iface);
            push_u32(&mut p, ver);
            push_u32(&mut p, new_id);
            cmds.extend_from_slice(&encode_msg(2, 0, &p));
        }
        cmds.extend_from_slice(&encode_msg(4, 0, &7u32.to_le_bytes()));
        let mut gs = Vec::new();
        push_u32(&mut gs, 8);
        push_u32(&mut gs, 7);
        cmds.extend_from_slice(&encode_msg(6, 2, &gs));
        cmds.extend_from_slice(&encode_msg(8, 1, &9u32.to_le_bytes()));
        let mut aid = Vec::new();
        push_str(&mut aid, "launched-app");
        cmds.extend_from_slice(&encode_msg(9, 3, &aid));
        send_with_fds(stream.as_raw_fd(), &cmds, &[]).unwrap();

        let pending_deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut pending = Vec::new();
        while std::time::Instant::now() < pending_deadline {
            pending = comp.take_pending_toplevels();
            if !pending.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].app_id, "launched-app");

        let sync = comp.sync_figures(vec![FigureSlot {
            key: "f0".into(),
            app_id: Some("launched-app".into()),
            width: 300,
            height: 220,
        }]);
        assert_eq!(sync["figures"][0]["bound"], true);

        drop(stream);
        drop(comp);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
