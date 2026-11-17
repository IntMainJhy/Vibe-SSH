LC_ALL=C
export LC_ALL
[ -r /proc/stat ] && [ -r /proc/meminfo ] && [ -r /proc/uptime ] || exit 69
first=$(awk '/^cpu / { t=0; for(i=2;i<=9;i++) t+=$i; printf "%.0f %.0f\n", t, $5+$6; exit }' /proc/stat) || exit
sleep 1
second=$(awk '/^cpu / { t=0; for(i=2;i<=9;i++) t+=$i; printf "%.0f %.0f\n", t, $5+$6; exit }' /proc/stat) || exit
cpu=$(printf '%s\n%s\n' "$first" "$second" | awk 'NR==1 {t=$1; idle=$2} NR==2 {d=$1-t; if(d<=0) print "null"; else printf "%.2f", 100*(1-($2-idle)/d)}') || exit
memory=$(awk '/^MemTotal:/ {t=$2} /^MemAvailable:/ {a=$2; available=1} /^SwapTotal:/ {s=$2} /^SwapFree:/ {f=$2} END {if(t<=0 || !available) exit 1; printf "\"memory_total\":%.0f,\"memory_available\":%.0f,\"swap_total\":%.0f,\"swap_free\":%.0f",t*1024,a*1024,s*1024,f*1024}' /proc/meminfo) || exit
storage=$(df -Pk / | awk 'NR==2 {printf "\"disk_total\":%.0f,\"disk_used\":%.0f,\"disk_available\":%.0f",$2*1024,$3*1024,$4*1024}') || exit
load=$(awk '{printf "[%s,%s,%s]",$1,$2,$3}' /proc/loadavg) || exit
uptime=$(awk '{printf "%.0f",$1}' /proc/uptime) || exit
cores=$(awk '/^processor/ {n++} END {print n+0}' /proc/cpuinfo) || exit
printf '{"cpu_percent":%s,"cores":%s,"uptime_s":%s,"load":%s,%s,%s}\n' "$cpu" "$cores" "$uptime" "$load" "$memory" "$storage"
