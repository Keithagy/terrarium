package store

import (
	"database/sql"
	"os"
)

type Job struct {
	ID   int
	Name string
}

func open() *sql.DB {
	db, _ := sql.Open("postgres", os.Getenv("DATABASE_URL"))
	return db
}

func ListJobs() []Job {
	db := open()
	rows, _ := db.Query("SELECT id, name FROM jobs")
	defer rows.Close()
	return nil
}
