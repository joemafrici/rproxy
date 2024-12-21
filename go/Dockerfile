# syntax=docker/dockerfile:1
FROM golang:alpine
COPY go.mod go.sum rproxy.go .
RUN go build
EXPOSE 3000
CMD ["./rproxy"]
