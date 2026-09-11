package main

import (
	"bufio"
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"flag"
	"fmt"
	"log"
	"os"
	"strings"

	"golang.org/x/term"

	"vclogg/server/internal/app"
	"vclogg/server/internal/config"
	"vclogg/server/internal/store"
)

func main() {
	if len(os.Args) < 2 {
		usage()
		os.Exit(2)
	}
	switch os.Args[1] {
	case "serve":
		serve(os.Args[2:])
	case "admin":
		admin(os.Args[2:])
	case "backup":
		backup(os.Args[2:])
	case "restore":
		restore(os.Args[2:])
	case "client-key":
		clientKey(os.Args[2:])
	default:
		usage()
		os.Exit(2)
	}
}
func usage() {
	fmt.Fprintln(os.Stderr, "usage: vclogg-server <serve|admin|backup|restore|client-key> [options]")
}
func load(configPath string) (config.Config, *store.Store) {
	cfg, err := config.Load(configPath)
	if err != nil {
		log.Fatal(err)
	}
	database, err := store.Open(context.Background(), cfg.DataDir)
	if err != nil {
		log.Fatal(err)
	}
	return cfg, database
}
func serve(args []string) {
	flags := flag.NewFlagSet("serve", flag.ExitOnError)
	configPath := flags.String("config", "./config.yaml", "configuration file")
	_ = flags.Parse(args)
	_ = app.WriteExampleConfig(*configPath)
	cfg, database := load(*configPath)
	defer database.Close()
	if err := app.ListenAndServe(database, cfg); err != nil {
		log.Fatal(err)
	}
}

func clientKey(args []string) {
	if len(args) == 1 && args[0] == "bundled" {
		if config.BundledClientKeyID == "" || config.BundledClientPublicKey == "" {
			log.Fatal("this server build does not contain a bundled client public key")
		}
		if err := json.NewEncoder(os.Stdout).Encode(map[string]string{
			"keyId":     config.BundledClientKeyID,
			"publicKey": config.BundledClientPublicKey,
		}); err != nil {
			log.Fatal(err)
		}
		return
	}
	if len(args) != 1 || args[0] != "generate" {
		log.Fatal("usage: vclogg-server client-key <generate|bundled>")
	}
	publicKey, privateKey, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		log.Fatal(err)
	}
	privateDER, err := x509.MarshalPKCS8PrivateKey(privateKey)
	if err != nil {
		log.Fatal(err)
	}
	fmt.Printf("public_key: %s\n", base64.RawURLEncoding.EncodeToString(publicKey))
	fmt.Printf("private_key: %s\n", base64.RawURLEncoding.EncodeToString(privateDER))
}
func admin(args []string) {
	if len(args) < 1 {
		log.Fatal("usage: vclogg-server admin <create|reset-password>")
	}
	flags := flag.NewFlagSet("admin", flag.ExitOnError)
	configPath := flags.String("config", "./config.yaml", "configuration file")
	username := flags.String("username", "admin", "administrator username")
	_ = flags.Parse(args[1:])
	_, database := load(*configPath)
	defer database.Close()
	password := readPassword()
	var err error
	if args[0] == "create" {
		err = app.CreateAdmin(context.Background(), database, *username, password)
	} else if args[0] == "reset-password" {
		err = app.ResetAdminPassword(context.Background(), database, *username, password)
	} else {
		log.Fatal("unknown admin command")
	}
	if err != nil {
		log.Fatal(err)
	}
	fmt.Println("administrator saved")
}
func readPassword() string {
	fd := int(os.Stdin.Fd())
	fmt.Print("Password: ")
	if term.IsTerminal(fd) {
		data, err := term.ReadPassword(fd)
		fmt.Println()
		if err != nil {
			log.Fatal(err)
		}
		return string(data)
	}
	line, err := bufio.NewReader(os.Stdin).ReadString('\n')
	if err != nil {
		log.Fatal(err)
	}
	return strings.TrimSpace(line)
}
func backup(args []string) {
	flags := flag.NewFlagSet("backup", flag.ExitOnError)
	configPath := flags.String("config", "./config.yaml", "configuration file")
	output := flags.String("output", "", "backup directory")
	_ = flags.Parse(args)
	cfg, database := load(*configPath)
	defer database.Close()
	target, err := database.Backup(context.Background(), app.ResolveBackupOutput(cfg.DataDir, *output), "vclogg")
	if err != nil {
		log.Fatal(err)
	}
	fmt.Println(target)
}
func restore(args []string) {
	flags := flag.NewFlagSet("restore", flag.ExitOnError)
	configPath := flags.String("config", "./config.yaml", "configuration file")
	input := flags.String("input", "", "backup database path")
	_ = flags.Parse(args)
	if *input == "" {
		log.Fatal("-input is required")
	}
	cfg, err := config.Load(*configPath)
	if err != nil {
		log.Fatal(err)
	}
	if err = store.Restore(*input, cfg.DataDir); err != nil {
		log.Fatal(err)
	}
	fmt.Println("database restored")
}
