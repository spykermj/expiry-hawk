FROM clux/muslrust:stable AS build

WORKDIR /build

RUN mkdir src; echo 'fn main() { panic!("Dummy Image Called!")}' > ./src/main.rs
COPY ["Cargo.toml", "Cargo.lock",  "./"]
RUN cargo build --release
COPY src src

#need to break the cargo cache
RUN touch ./src/main.rs

RUN cargo build --release; export BINARY=`find target -type f -name expiry-hawk`; cp ${BINARY} .

FROM scratch

ARG workdir=/app/bin

WORKDIR ${workdir}

COPY --from=build /etc/passwd /etc/passwd
COPY --from=build /etc/group /etc/group
COPY --from=build /build/expiry-hawk .
USER nobody:nogroup
ENTRYPOINT ["/app/bin/expiry-hawk"]
