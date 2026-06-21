FROM scratch
COPY t1ds_signaling_rs /t1ds_signaling_rs
EXPOSE 3000
ENTRYPOINT ["/t1ds_signaling_rs"]