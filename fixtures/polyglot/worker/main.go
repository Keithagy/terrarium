package main

import (
	"encoding/json"
	"net/http"

	"example.com/worker/store"
)

func handleJobs(w http.ResponseWriter, r *http.Request) {
	jobs := store.ListJobs()
	json.NewEncoder(w).Encode(jobs)
}

func main() {
	http.HandleFunc("/api/jobs", handleJobs)
	http.ListenAndServe(":8080", nil)
}
