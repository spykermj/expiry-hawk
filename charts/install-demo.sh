#!/bin/sh
#

for ns in one two three four five six seven eight nine ten; do
    for i in alpha bravo charlie delta echo; do
        helm -n $ns install $i --set expiryDays=`shuf -i 0-90 -n 1` --create-namespace ./demo
    done
done
