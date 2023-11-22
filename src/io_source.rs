use std::ops::{Deref, DerefMut};
#[cfg(unix)]
use std::os::unix::io::AsRawFd;
#[cfg(target_os = "wasi")]
use std::os::wasi::io::AsRawFd;
#[cfg(windows)]
use std::os::windows::io::AsRawSocket;
#[cfg(debug_assertions)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{fmt, io};

use crate::sys::IoSourceState;
use crate::{event, Interest, Registry, Token};

/// 对事件源RawFd和RawSocket进行包装,统一实现[`event::Source`].<br>
/// Adapter for a [`RawFd`] or [`RawSocket`] providing an [`event::Source`]
/// implementation.
///
/// `IoSource`实现了[`event::Source`]可以注册任意FD或socket的包装到[`Poll`]中.<br>
/// `IoSource` enables registering any FD or socket wrapper with [`Poll`].
///
/// 目前仅提供TCP,UDP,UDS的实现.MIO支持对可通过底层OS选择器注册的任意的FD或socket进行注册.<br>
/// While only implementations for TCP, UDP, and UDS (Unix only) are provided,
/// Mio supports registering any FD or socket that can be registered with the
/// underlying OS selector. `IoSource` provides the necessary bridge.
///
/// [`RawFd`]: std::os::unix::io::RawFd
/// [`RawSocket`]: std::os::windows::io::RawSocket
///
/// # 注意, Notes
/// 
/// 为了正确处理注册和事件,所有IO操作(read,write,etc)必须通过[`do_io`]入口执行,以确保内部状态做相应更新.<br>
/// To handle the registrations and events properly **all** I/O operations (such
/// as `read`, `write`, etc.) must go through the [`do_io`] method to ensure the
/// internal state is updated accordingly.
///
/// [`Poll`]: crate::Poll
/// [`do_io`]: IoSource::do_io
///
/// # 例子, Examples
///
/// Basic usage.
///
/// ```
/// # use std::error::Error;
/// # fn main() -> Result<(), Box<dyn Error>> {
/// use mio::{Interest, Poll, Token};
/// use mio::IoSource;
///
/// use std::net;
///
/// let poll = Poll::new()?;
///
/// // Bind a std TCP listener.
/// let listener = net::TcpListener::bind("127.0.0.1:0")?;
/// // Wrap it in the `IoSource` type.
/// let mut listener = IoSource::new(listener);
///
/// // Register the listener.
/// poll.registry().register(&mut listener, Token(0), Interest::READABLE)?;
/// #     Ok(())
/// # }
/// ```
pub struct IoSource<T> {
    /// 单元值,主要为IoSource类型提供一些辅助方法
    /// - do_io(f,io): 执行io操作,f(io)..所有io操作都抽象为了一个统一入口do_io(op,io)
    /// - register,reregister,deregister: 通过Registry注册入口提供的事件注册方法注册事件(fd,token,interiest)
    state: IoSourceState,

    /// 被包装的真实IO事件源
    inner: T,

    /// IoSource与Selector的关联关系,用于调试
    #[cfg(debug_assertions)]
    selector_id: SelectorId,
}

impl<T> IoSource<T> {
    /// Create a new `IoSource`.
    pub fn new(io: T) -> IoSource<T> {
        IoSource {
            state: IoSourceState::new(),
            inner: io,
            #[cfg(debug_assertions)]
            selector_id: SelectorId::new(),
        }
    }

    /// 执行一个IO操作,确保套接字在遇到了WouldBlock错误后可以接收更多事件.<br>
    /// Execute an I/O operations ensuring that the socket receives more events
    /// if it hits a [`WouldBlock`] error.
    ///
    /// # 注意, Notes
    ///
    /// 所有IO操作都必须通过这个统一入口执行,确保套接字在WouldBlock错误之后如果重新就绪,用户能继续接收事件.<br>
    /// This method is required to be called for **all** I/O operations to
    /// ensure the user will receive events once the socket is ready again after
    /// returning a [`WouldBlock`] error.
    ///
    ///
    /// [`WouldBlock`]: io::ErrorKind::WouldBlock
    pub fn do_io<F, R>(&self, f: F) -> io::Result<R>
    where
        F: FnOnce(&T) -> io::Result<R>,
    {
        self.state.do_io(f, &self.inner)
    }

    /// 剥出内部事件源.<br>
    /// Returns the I/O source, dropping the state.
    ///
    /// # 注意
    /// 为了确保不再收到更多事件,先要执行deregister
    ///
    /// # Notes
    ///
    /// To ensure no more events are to be received for this I/O source first
    /// [`deregister`] it.
    ///
    /// [`deregister`]: Registry::deregister
    pub fn into_inner(self) -> T {
        self.inner
    }
}

/// 使用此方法时要注意,所有可能阻塞的IO操作必须先执行do_io
/// Be careful when using this method. All I/O operations that may block must go
/// through the [`do_io`] method.
///
/// [`do_io`]: IoSource::do_io
impl<T> Deref for IoSource<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

/// 使用此方法时要注意,所有可能阻塞的IO操作必须先执行do_io
/// Be careful when using this method. All I/O operations that may block must go
/// through the [`do_io`] method.
///
/// [`do_io`]: IoSource::do_io
impl<T> DerefMut for IoSource<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

#[cfg(unix)]
impl<T> event::Source for IoSource<T>
where
    T: AsRawFd,
{
    fn register(
        &mut self,
        registry: &Registry,
        token: Token,
        interests: Interest,
    ) -> io::Result<()> {
        #[cfg(debug_assertions)]
        self.selector_id.associate(registry)?;
        self.state
            .register(registry, token, interests, self.inner.as_raw_fd())
    }

    fn reregister(
        &mut self,
        registry: &Registry,
        token: Token,
        interests: Interest,
    ) -> io::Result<()> {
        #[cfg(debug_assertions)]
        self.selector_id.check_association(registry)?;
        self.state
            .reregister(registry, token, interests, self.inner.as_raw_fd())
    }

    fn deregister(&mut self, registry: &Registry) -> io::Result<()> {
        #[cfg(debug_assertions)]
        self.selector_id.remove_association(registry)?;
        self.state.deregister(registry, self.inner.as_raw_fd())
    }
}

#[cfg(windows)]
impl<T> event::Source for IoSource<T>
where
    T: AsRawSocket,
{
    fn register(
        &mut self,
        registry: &Registry,
        token: Token,
        interests: Interest,
    ) -> io::Result<()> {
        #[cfg(debug_assertions)]
        self.selector_id.associate(registry)?;
        self.state
            .register(registry, token, interests, self.inner.as_raw_socket())
    }

    fn reregister(
        &mut self,
        registry: &Registry,
        token: Token,
        interests: Interest,
    ) -> io::Result<()> {
        #[cfg(debug_assertions)]
        self.selector_id.check_association(registry)?;
        self.state.reregister(registry, token, interests)
    }

    fn deregister(&mut self, _registry: &Registry) -> io::Result<()> {
        #[cfg(debug_assertions)]
        self.selector_id.remove_association(_registry)?;
        self.state.deregister()
    }
}

#[cfg(target_os = "wasi")]
impl<T> event::Source for IoSource<T>
where
    T: AsRawFd,
{
    fn register(
        &mut self,
        registry: &Registry,
        token: Token,
        interests: Interest,
    ) -> io::Result<()> {
        #[cfg(debug_assertions)]
        self.selector_id.associate(registry)?;
        registry
            .selector()
            .register(self.inner.as_raw_fd() as _, token, interests)
    }

    fn reregister(
        &mut self,
        registry: &Registry,
        token: Token,
        interests: Interest,
    ) -> io::Result<()> {
        #[cfg(debug_assertions)]
        self.selector_id.check_association(registry)?;
        registry
            .selector()
            .reregister(self.inner.as_raw_fd() as _, token, interests)
    }

    fn deregister(&mut self, registry: &Registry) -> io::Result<()> {
        #[cfg(debug_assertions)]
        self.selector_id.remove_association(registry)?;
        registry.selector().deregister(self.inner.as_raw_fd() as _)
    }
}

impl<T> fmt::Debug for IoSource<T>
where
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}

/// 将IO事件源IoSource与事件监听器Poll/Registry/Selector进行关联
/// Used to associate an `IoSource` with a `sys::Selector`.
#[cfg(debug_assertions)]
#[derive(Debug)]
struct SelectorId {
    id: AtomicUsize,
}

#[cfg(debug_assertions)]
impl SelectorId {
    /// Value of `id` if `SelectorId` is not associated with any
    /// `sys::Selector`. Valid selector ids start at 1.
    const UNASSOCIATED: usize = 0;

    /// Create a new `SelectorId`.
    const fn new() -> SelectorId {
        SelectorId {
            id: AtomicUsize::new(Self::UNASSOCIATED),
        }
    }

    /// 与Registry进行关联,将Registry的ID设置给自己
    /// 本ID的值必须是初始值,如果不是初始值说明已经关联过Registry,则返回错误
    /// Associate an I/O source with `registry`, returning an error if its
    /// already registered.
    fn associate(&self, registry: &Registry) -> io::Result<()> {
        let registry_id = registry.selector().id();
        let previous_id = self.id.swap(registry_id, Ordering::AcqRel);

        if previous_id == Self::UNASSOCIATED {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "I/O source already registered with a `Registry`",
            ))
        }
    }

    /// 校验是否与Registry有关联关系
    /// Check the association of an I/O source with `registry`, returning an
    /// error if its registered with a different `Registry` or not registered at
    /// all.
    fn check_association(&self, registry: &Registry) -> io::Result<()> {
        let registry_id = registry.selector().id();
        let id = self.id.load(Ordering::Acquire);

        if id == registry_id {
            Ok(())
        } else if id == Self::UNASSOCIATED {
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                "I/O source not registered with `Registry`",
            ))
        } else {
            Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "I/O source already registered with a different `Registry`",
            ))
        }
    }

    /// 解除关联,将自己置为初始值
    /// Remove a previously made association from `registry`, returns an error
    /// if it was not previously associated with `registry`.
    fn remove_association(&self, registry: &Registry) -> io::Result<()> {
        let registry_id = registry.selector().id();
        let previous_id = self.id.swap(Self::UNASSOCIATED, Ordering::AcqRel);

        if previous_id == registry_id {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                "I/O source not registered with `Registry`",
            ))
        }
    }
}

#[cfg(debug_assertions)]
impl Clone for SelectorId {
    fn clone(&self) -> SelectorId {
        SelectorId {
            id: AtomicUsize::new(self.id.load(Ordering::Acquire)),
        }
    }
}
