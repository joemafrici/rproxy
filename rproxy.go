package main

import (
	"log"
	"net/http"
	"net/http/httputil"
	"net/url"
	"strings"
)

func main() {
	url, err := url.Parse("http://gojoe:7002")
	if err != nil {
		log.Fatal(err.Error())
	}
	uploaderUrl, err := url.Parse("http://uploader:80")
	if err != nil {
		log.Fatal(err.Error())
	}
	imgservUrl, err := url.Parse("http://imgserv:80")
	if err != nil {
		log.Fatal(err.Error())
	}
	helloWorldUrl, err := url.Parse("http://hello_world:80")
	if err != nil {
		log.Fatal(err.Error())
	}
	rp := httputil.NewSingleHostReverseProxy(url)
	helloWorldRp := httputil.NewSingleHostReverseProxy(helloWorldUrl)
	uploaderProxy := httputil.NewSingleHostReverseProxy(uploaderUrl)
	uploaderProxy.Director = func(r *http.Request) {
		r.URL.Scheme = uploaderUrl.Scheme
		r.URL.Host = uploaderUrl.Host

		r.URL.Path = strings.TrimPrefix(r.URL.Path, "/uploader")
		if r.URL.Path == "" {
			r.URL.Path = "/"
		}
	}
	imgservProxy := httputil.NewSingleHostReverseProxy(imgservUrl)
	imgservProxy.Director = func(r *http.Request) {
		r.URL.Scheme = uploaderUrl.Scheme
		r.URL.Host = uploaderUrl.Host

		r.URL.Path = strings.TrimPrefix(r.URL.Path, "/imgserv")
		if r.URL.Path == "" {
			r.URL.Path = "/"
		}
	}
	mux := http.NewServeMux()
	mux.HandleFunc("/uploader/", logging(uploaderProxy.ServeHTTP))
	mux.HandleFunc("/imgserv/", logging(imgservProxy.ServeHTTP))
	mux.HandleFunc("/hello_world", logging(helloWorldRp.ServeHTTP))
	mux.HandleFunc("/", logging(rp.ServeHTTP))

	addr := "0.0.0.0:3000"
	log.Println("Server listening on addr ", addr)
	log.Fatal(http.ListenAndServe(addr, mux))
}

//	func handler(w http.ResponseWriter, r *http.Request) {
//		w.Write([]byte("Hello from server!"))
//	}
func logging(f http.HandlerFunc) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		log.Printf("rproxy received request %v for %v from %v\n", r.Method, r.RequestURI, r.RemoteAddr)
		if r.Header.Get("HX-Request") == "true" {
			log.Println("is an htmx request")
		}
		f(w, r)
	}
}
