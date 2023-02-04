# RSFU

A WebRTC selective forwding unit written in Rust, rewrite [ion-sfu](https://github.com/pion/ion-sfu).


# Build and Run

#### Clone
    git clone https://github.com/harlanc/rsfu.git
    
#### Build
    cargo build

#### Run the jrpc2-server
    
    ./target/debug/jrpc2-server 
    
#### Open the client 

Open the rsfu/third-party/echotest-jsonrpc/index.html from the chrome browser, and click **Start**, your audio/video can be forward from left side to the right side.


# Dependencies

- [webrtc-rs](https://github.com/webrtc-rs/webrtc)
- [jsonrpc2-rs](https://github.com/harlanc/jsonrpc2-rs)