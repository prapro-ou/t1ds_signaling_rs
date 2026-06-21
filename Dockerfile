FROM scratch
COPY target/x86_64-unknown-linux-musl/release/t1ds_signaling_rs /t1ds_signaling_rs
EXPOSE 3000
ENTRYPOINT ["/t1ds_signaling_rs"]