package main

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net/http"
	"net/http/httputil"
	"net/url"
	"strings"
	"sync"
	"time"

	"github.com/docker/docker/api/types"
	"github.com/docker/docker/api/types/container"
	dockerclient "github.com/docker/docker/client"
)

type Service struct {
	Name string
}
type ServiceProxy struct {
	Current      *Service
	mu           sync.RWMutex
	DockerClient *dockerclient.Client
}

func NewServiceProxy(dockerClient *dockerclient.Client, currentServiceName string) *ServiceProxy {
	return &ServiceProxy{
		DockerClient: dockerClient,
		Current: &Service{
			Name: currentServiceName,
		},
	}
}

func (s *ServiceProxy) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	s.mu.RLock()
	currentService := s.Current
	s.mu.RUnlock()

	proxy := httputil.NewSingleHostReverseProxy(&url.URL{
		Scheme: "http",
		Host:   fmt.Sprintf("%s:80", currentService.Name),
	})
	proxy.ServeHTTP(w, r)
}

func (s *ServiceProxy) HandleSwitch(w http.ResponseWriter, r *http.Request) {
	if r.Method != "POST" {
		http.Error(w, "Wrong verb", http.StatusMethodNotAllowed)
		return
	}

	decoder := json.NewDecoder(r.Body)
	var switchRequest struct {
		New string `json:"new"`
	}
	if err := decoder.Decode(&switchRequest); err != nil {
		if err == io.EOF {
			http.Error(w, "No body provided", http.StatusBadRequest)
			return
		}
	}
	new := &Service{Name: switchRequest.New}

	log.Printf("Determining if %s is healthy\n", new.Name)

	isHealthy := false
	for ii := 0; ii < 10; ii++ {
		log.Println("checking health...")
		if healthy, err := containerIsHealthy(s.DockerClient, new.Name); err != nil {
			if healthy {
				isHealthy = true
				break
			}

			if err.Error() == "unhealthy" {
				http.Error(w, "Container is not healthy", http.StatusInternalServerError)
				return
			}
		}

		time.Sleep(time.Millisecond * 500)
	}

	if isHealthy == false {
		http.Error(w, "Container failed to become healthy", http.StatusInternalServerError)
		return
	}

	log.Println("Container is healthy")

	s.mu.Lock()
	old := s.Current
	s.Current = new
	s.mu.Unlock()

	// TODO: wait for requests to drain
	fmt.Printf("Attempting to stop container %s\n", old.Name)
	err := s.DockerClient.ContainerStop(context.TODO(), old.Name, container.StopOptions{})
	if err != nil {
		log.Printf("%v\n", err)
		http.Error(w, "Failed to stop container", http.StatusInternalServerError)
		return
	}

	log.Printf("Attempting to remove container %s\n", old.Name)
	err = s.DockerClient.ContainerRemove(context.TODO(), old.Name, container.RemoveOptions{})
	if err != nil {
		log.Printf("%v\n", err)
		http.Error(w, "Failed to remove container", http.StatusInternalServerError)
		return
	}
}

func containerIsHealthy(dockerClient *dockerclient.Client, containerName string) (bool, error) {
	containerInfo, err := dockerClient.ContainerInspect(context.TODO(), containerName)
	if err != nil {
		log.Println("Failed to check container health")
		return false, err
	}
	if containerInfo.State.Health != nil {
		if containerInfo.State.Health.Status == types.Healthy {
			return true, nil
		}
		if containerInfo.State.Health.Status == types.Unhealthy {
			return false, fmt.Errorf("unhealthy")
		}
	}
	return false, nil
}

func main() {
	dockerClient, err := dockerclient.NewClientWithOpts(dockerclient.FromEnv)
	if err != nil {
		panic(err)
	}
	defer dockerClient.Close()

	helloWorldProxy := NewServiceProxy(dockerClient, "hello_world_1")

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
	//helloWorldUrl, err := url.Parse("http://hello_world:80")
	//if err != nil {
	//	log.Fatal(err.Error())
	//}
	rp := httputil.NewSingleHostReverseProxy(url)
	//helloWorldRp := httputil.NewSingleHostReverseProxy(helloWorldUrl)
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
	mux.HandleFunc("/api/switch", logging(helloWorldProxy.HandleSwitch))
	mux.HandleFunc("/uploader/", logging(uploaderProxy.ServeHTTP))
	mux.HandleFunc("/imgserv/", logging(imgservProxy.ServeHTTP))
	mux.HandleFunc("/hello_world", logging(helloWorldProxy.ServeHTTP))
	mux.HandleFunc("/", logging(rp.ServeHTTP))

	addr := "0.0.0.0:3000"
	log.Println("Server listening on addr ", addr)
	log.Fatal(http.ListenAndServe(addr, mux))
}

func handleSwitch(w http.ResponseWriter, r *http.Request) {
	if r.Method != "POST" {
		http.Error(w, "Wrong verb", http.StatusMethodNotAllowed)
		return
	}

	decoder := json.NewDecoder(r.Body)
	var switchRequest struct {
		ServiceName string `json:"serviceName"`
		Port        int    `json:"port"`
	}
	if err := decoder.Decode(&switchRequest); err != nil {
		if err == io.EOF {
			http.Error(w, "No body provided", http.StatusBadRequest)
			return
		}
	}
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
