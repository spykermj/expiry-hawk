#!/bin/sh
#

kind load docker-image jeremyspykerman/expiry-hawk:latest
helm -n expiry-hawk install expiry-hawk --create-namespace ./expiry-hawk
