use std::{sync::Arc, time::Duration};

use mio::{Events, Poll, Token, Waker};

fn main() -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
    // 用户实现唤醒器的事件TOKEN
    const WAKE_TOKEN: Token = Token(1);

    // 创建epoll实例
    let mut poll = Poll::new()?;
    // 创建存储就绪事件的数组
    let mut events = Events::with_capacity(2);

    // 创建用于唤醒poll实例的Waker,此时会注册一个eventfd+readable事件监听到poll中
    //
    // 唤醒的原理:
    // 1.poll.poll()方法如果未设置超时,当没有轮询到就绪事件时则阻塞
    // 2.当调用waker.wake()时会导致eventfd+readable事件就绪
    // 3.poll.poll()监测到了eventfd+readable就绪事件就解除了阻塞
    //
    // 让eventfd+readable事件就绪原理:
    // 1.创建一个特殊的虚拟文件句柄eventfd
    // 2.向虚拟文件句柄eventfd中写入数据,则会导致readable事件就绪
    let waker = Arc::new(Waker::new(poll.registry(), WAKE_TOKEN)?);
    let waker1 = Arc::clone(&waker);
    // 模拟在其它线程唤醒poll实例
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(5));
        waker1.wake().expect("unable to wake");
    });

    // 轮询就绪事件,没有就绪事件则""阻塞""等待
    // 如果有任何事件就绪(包括注册到poll中的用于唤醒器的eventfd句柄对应的readable事件)
    poll.poll(&mut events, None)?;

    assert!(!events.is_empty());
    let waker_event = events.iter().next().unwrap();
    assert!(waker_event.is_readable());
    assert_eq!(waker_event.token(), WAKE_TOKEN);

    Ok(())
}
