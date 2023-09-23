#!/bin/sh
#

for ns in one two three four five six seven eight nine ten; do
    for i in alpha bravo charlie delta echo; do
        helm -n $ns install $i --create-namespace ./demo
    done
done
