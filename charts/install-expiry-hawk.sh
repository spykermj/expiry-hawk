#!/bin/sh
#

kind load docker-image ghcr.io/spykermj/expiry-hawk:0.1.0
helm -n expiry-hawk install expiry-hawk --create-namespace ./expiry-hawk
